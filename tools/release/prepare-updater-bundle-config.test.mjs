import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { prepareUpdaterBundleConfig } from './prepare-updater-bundle-config.mjs';
import { REPOSITORY_ROOT } from './release-common.mjs';
import { UPDATER_ENDPOINT } from './tauri-generation.mjs';

function git(root, args) {
  return execFileSync('git', args, {
    cwd: root,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

async function fixture(t) {
  const parent = await mkdtemp(join(await realpath(tmpdir()), 'nexusops-public-bundle-config-'));
  t.after(() => rm(parent, { recursive: true, force: true }));
  const root = join(parent, 'source');
  await mkdir(join(root, 'apps/desktop/src-tauri'), { recursive: true });
  await mkdir(join(root, 'apps/desktop/src-tauri/capabilities'), { recursive: true });
  await mkdir(join(root, 'apps/desktop/src-tauri/src'), { recursive: true });
  await mkdir(join(root, 'docs/release/keys'), { recursive: true });
  const publicKey = await readFile(join(REPOSITORY_ROOT, 'docs/release/keys/nexusops-updater.pub'), 'utf8');
  await writeFile(join(root, 'docs/release/keys/nexusops-updater.pub'), publicKey);
  await writeFile(join(root, 'package.json'), JSON.stringify({ dependencies: {}, devDependencies: {} }));
  await writeFile(join(root, 'apps/desktop/package.json'), JSON.stringify({
    dependencies: { '@tauri-apps/api': '2.12.0' },
    devDependencies: { '@tauri-apps/cli': '2.12.0' },
  }));
  await writeFile(join(root, 'package-lock.json'), JSON.stringify({ packages: {
    'apps/desktop': {
      dependencies: { '@tauri-apps/api': '2.12.0' },
      devDependencies: { '@tauri-apps/cli': '2.12.0' },
    },
    'node_modules/@tauri-apps/api': { version: '2.12.0' },
    'node_modules/@tauri-apps/cli': { version: '2.12.0' },
    'node_modules/@tauri-apps/cli-win32-x64-msvc': { version: '2.12.0' },
  } }));
  await writeFile(join(root, 'apps/desktop/src-tauri/Cargo.toml'),
    '[dependencies]\ntauri = { version = "=2.12.0", features = [] }\ntauri-plugin-updater = { version = "=2.13.1" }\nreqwest = { version = "=0.13.5" }\nminisign-verify = { version = "=0.2.5" }\nbase64 = { version = "=0.22.1" }\nsemver = { version = "=1.0.28" }\n[build-dependencies]\ntauri-build = { version = "=2.7.0", features = [] }\n');
  await writeFile(join(root, 'Cargo.lock'),
    '[[package]]\nname = "tauri"\nversion = "2.12.0"\n\n[[package]]\nname = "tauri-build"\nversion = "2.7.0"\n\n[[package]]\nname = "tauri-plugin-updater"\nversion = "2.13.1"\n\n[[package]]\nname = "reqwest"\nversion = "0.13.5"\n\n[[package]]\nname = "minisign-verify"\nversion = "0.2.5"\n\n[[package]]\nname = "base64"\nversion = "0.22.1"\n\n[[package]]\nname = "semver"\nversion = "1.0.28"\n');
  await writeFile(join(root, 'apps/desktop/src-tauri/tauri.conf.json'),
    JSON.stringify({ bundle: { createUpdaterArtifacts: true }, plugins: { updater: {
      pubkey: publicKey,
      endpoints: [UPDATER_ENDPOINT],
      requireSignedVersion: true,
      allowDowngrades: false,
      dangerousInsecureTransportProtocol: false,
      dangerousAcceptInvalidCerts: false,
      dangerousAcceptInvalidHostnames: false,
    } } }));
  await writeFile(join(root, 'apps/desktop/src-tauri/capabilities/main.json'),
    JSON.stringify({ permissions: ['allow-get-update-state', 'allow-check-for-update', 'allow-download-announced-update'] }));
  await writeFile(join(root, 'apps/desktop/src-tauri/src/update_download.rs'), [
    'MAX_ARTIFACT_BYTES: usize = 134_217_728',
    'CONNECT_TIMEOUT: Duration = Duration::from_secs(10)',
    'DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300)',
    'MAX_REDIRECTS: usize = 3',
    '"/unrealbg/NexusOps/releases/download/"',
    '"release-assets.githubusercontent.com"',
    '"objects.githubusercontent.com"',
    '.https_only(true)',
    '.no_proxy()',
    '.retry(reqwest::retry::never())',
  ].join('\n'));
  await writeFile(join(root, 'apps/desktop/src-tauri/src/updates.rs'),
    'bounded native update state');
  git(root, ['init', '-q']);
  git(root, ['add', '.']);
  git(root, ['-c', 'user.name=NexusOps Test', '-c', 'user.email=test@example.invalid',
    'commit', '-qm', 'synthetic signed candidate source']);
  return { root, parent, publicKey, head: git(root, ['rev-parse', 'HEAD']) };
}

test('temporary bundle overlay contains only the reviewed public key', async (t) => {
  const { root, parent, publicKey, head } = await fixture(t);
  const output = join(parent, 'bundle-config.json');
  assert.equal(await prepareUpdaterBundleConfig(output, root, { GITHUB_SHA: head }), output);
  assert.deepEqual(JSON.parse(await readFile(output, 'utf8')),
    { plugins: { updater: { pubkey: publicKey } } });
  const runtime = JSON.parse(await readFile(join(root, 'apps/desktop/src-tauri/tauri.conf.json'), 'utf8'));
  assert.equal(runtime.plugins.updater.pubkey, publicKey);
  assert.deepEqual(runtime.plugins.updater.endpoints, [UPDATER_ENDPOINT]);
  assert.equal(runtime.plugins.updater.requireSignedVersion, true);
  await assert.rejects(prepareUpdaterBundleConfig(output, root, { GITHUB_SHA: head }),
    /EEXIST/);
});

test('temporary overlay refuses wrong or dirty source', async (t) => {
  const { root, parent, head } = await fixture(t);
  await assert.rejects(prepareUpdaterBundleConfig(join(parent, 'wrong.json'), root,
    { GITHUB_SHA: 'f'.repeat(40) }), /GITHUB_SHA does not match/);
  await writeFile(join(root, 'unexpected.txt'), 'untracked input');
  await assert.rejects(prepareUpdaterBundleConfig(join(parent, 'dirty.json'), root,
    { GITHUB_SHA: head }), /working tree is dirty/);
});

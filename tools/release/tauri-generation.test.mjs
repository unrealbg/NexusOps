import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { verifyTauriGeneration } from './tauri-generation.mjs';

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'nexusops-tauri-generation-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'apps/desktop/src-tauri'), { recursive: true });
  await writeFile(
    join(root, 'apps/desktop/package.json'),
    JSON.stringify({
      dependencies: { '@tauri-apps/api': '2.12.0' },
      devDependencies: { '@tauri-apps/cli': '2.12.0' },
    }),
  );
  await writeFile(
    join(root, 'package-lock.json'),
    JSON.stringify({
      packages: {
        'apps/desktop': {
          dependencies: { '@tauri-apps/api': '2.12.0' },
          devDependencies: { '@tauri-apps/cli': '2.12.0' },
        },
        'node_modules/@tauri-apps/api': { version: '2.12.0' },
        'node_modules/@tauri-apps/cli': { version: '2.12.0' },
        'node_modules/@tauri-apps/cli-win32-x64-msvc': { version: '2.12.0' },
      },
    }),
  );
  await writeFile(
    join(root, 'apps/desktop/src-tauri/Cargo.toml'),
    '[dependencies]\ntauri = { version = "=2.12.0", features = [] }\n[build-dependencies]\ntauri-build = { version = "=2.7.0", features = [] }\n',
  );
  await writeFile(
    join(root, 'Cargo.lock'),
    '[[package]]\nname = "tauri"\nversion = "2.12.0"\n\n[[package]]\nname = "tauri-build"\nversion = "2.7.0"\n',
  );
  await writeFile(
    join(root, 'apps/desktop/src-tauri/tauri.conf.json'),
    JSON.stringify({ bundle: { createUpdaterArtifacts: true } }),
  );
  return root;
}

async function changeJson(root, path, change) {
  const full = join(root, path);
  const value = JSON.parse(await readFile(full, 'utf8'));
  change(value);
  await writeFile(full, JSON.stringify(value));
}

test('required Tauri 2.12 generation and artifact setting pass', async (t) => {
  const root = await fixture(t);
  assert.deepEqual(await verifyTauriGeneration(root), {
    api: '2.12.0',
    cli: '2.12.0',
    tauri: '2.12.0',
    tauriBuild: '2.7.0',
  });
});

test('direct and locked npm Tauri versions must match exactly', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'apps/desktop/package.json', (p) => { p.dependencies['@tauri-apps/api'] = '2.11.1'; });
  await assert.rejects(verifyTauriGeneration(root), /direct Tauri API/);
  await changeJson(root, 'apps/desktop/package.json', (p) => { p.dependencies['@tauri-apps/api'] = '2.12.0'; });
  await changeJson(root, 'package-lock.json', (p) => { p.packages['node_modules/@tauri-apps/cli-win32-x64-msvc'].version = '2.11.4'; });
  await assert.rejects(verifyTauriGeneration(root), /cli-win32-x64-msvc/);
});

test('direct Rust Tauri and tauri-build must be exactly pinned', async (t) => {
  const root = await fixture(t);
  const file = join(root, 'apps/desktop/src-tauri/Cargo.toml');
  await writeFile(file, '[dependencies]\ntauri = { version = "2.12.0" }\n[build-dependencies]\ntauri-build = { version = "=2.7.0" }\n');
  await assert.rejects(verifyTauriGeneration(root), /exactly pinned/);
  await writeFile(file, '[dependencies]\ntauri = { version = "=2.12.0" }\n[build-dependencies]\ntauri-build = { version = "=2.6.3" }\n');
  await assert.rejects(verifyTauriGeneration(root), /tauri-build/);
});

test('resolved Cargo versions must match direct Tauri generation', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'Cargo.lock'), '[[package]]\nname = "tauri"\nversion = "2.11.5"\n\n[[package]]\nname = "tauri-build"\nversion = "2.7.0"\n');
  await assert.rejects(verifyTauriGeneration(root), /locked Rust Tauri/);
});

test('updater artifacts enabled without runtime updater plugin config', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => {
    config.bundle.createUpdaterArtifacts = false;
  });
  await assert.rejects(verifyTauriGeneration(root), /createUpdaterArtifacts/);
  await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => {
    config.bundle.createUpdaterArtifacts = true;
    config.plugins = { updater: { pubkey: 'synthetic' } };
  });
  await assert.rejects(verifyTauriGeneration(root), /runtime plugin configuration/);
});

test('signed bundling rejects hooks and implicit platform config overlays', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => {
    config.build = { beforeBundleCommand: 'npm run unrelated-script' };
  });
  await assert.rejects(verifyTauriGeneration(root), /beforeBundleCommand/);
  await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => {
    delete config.build;
  });
  await writeFile(join(root, 'apps/desktop/src-tauri/tauri.windows.conf.json'), '{}');
  await assert.rejects(verifyTauriGeneration(root), /platform-specific Tauri configuration/);
});

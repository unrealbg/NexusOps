import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { REPOSITORY_ROOT } from './release-common.mjs';
import { UPDATER_ENDPOINT, verifyTauriGeneration } from './tauri-generation.mjs';

const PUBLIC_KEY_FILE = 'docs/release/keys/nexusops-updater.pub';

async function reviewedPublicKey() {
  return readFile(join(REPOSITORY_ROOT, PUBLIC_KEY_FILE), 'utf8');
}

function updaterConfig(pubkey) {
  return {
    pubkey,
    endpoints: [UPDATER_ENDPOINT],
    requireSignedVersion: true,
    allowDowngrades: false,
    dangerousInsecureTransportProtocol: false,
    dangerousAcceptInvalidCerts: false,
    dangerousAcceptInvalidHostnames: false,
  };
}

async function fixture(t) {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'nexusops-tauri-generation-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'apps/desktop/src-tauri'), { recursive: true });
  await mkdir(join(root, 'apps/desktop/src-tauri/capabilities'), { recursive: true });
  await mkdir(join(root, 'apps/desktop/src-tauri/src'), { recursive: true });
  await mkdir(join(root, 'docs/release/keys'), { recursive: true });
  const pubkey = await reviewedPublicKey();
  await writeFile(join(root, PUBLIC_KEY_FILE), pubkey);
  await writeFile(join(root, 'package.json'), JSON.stringify({ dependencies: {}, devDependencies: {} }));
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
    '[dependencies]\ntauri = { version = "=2.12.0", features = [] }\ntauri-plugin-updater = { version = "=2.13.1" }\nreqwest = { version = "=0.13.5" }\nminisign-verify = { version = "=0.2.5" }\nbase64 = { version = "=0.22.1" }\nsemver = { version = "=1.0.28" }\n[build-dependencies]\ntauri-build = { version = "=2.7.0", features = [] }\n',
  );
  await writeFile(
    join(root, 'Cargo.lock'),
    '[[package]]\nname = "tauri"\nversion = "2.12.0"\n\n[[package]]\nname = "tauri-build"\nversion = "2.7.0"\n\n[[package]]\nname = "tauri-plugin-updater"\nversion = "2.13.1"\n\n[[package]]\nname = "reqwest"\nversion = "0.13.5"\n\n[[package]]\nname = "minisign-verify"\nversion = "0.2.5"\n\n[[package]]\nname = "base64"\nversion = "0.22.1"\n\n[[package]]\nname = "semver"\nversion = "1.0.28"\n',
  );
  await writeFile(
    join(root, 'apps/desktop/src-tauri/tauri.conf.json'),
    JSON.stringify({ bundle: { createUpdaterArtifacts: true }, plugins: { updater: updaterConfig(pubkey) } }),
  );
  await writeFile(join(root, 'apps/desktop/src-tauri/capabilities/main.json'),
    JSON.stringify({ permissions: ['allow-list-hosts', 'allow-get-update-state', 'allow-check-for-update', 'allow-download-announced-update', 'allow-install-verified-update'] }));
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
  await writeFile(join(root, 'apps/desktop/src-tauri/src/updates.rs'), [
    'bounded native update state',
    '.restart_after_install(false)',
    '.on_before_exit(|| {})',
    'RetainedTauriUpdate::new(update)',
    'installation_supported() && verified_artifact_id.is_some()',
  ].join('\n'));
  await writeFile(join(root, 'apps/desktop/src-tauri/src/update_install.rs'), [
    '#[cfg(all(windows, any(target_arch = "x86_64", target_arch = "aarch64")))]',
    'spawn_blocking',
    'retained_update.install(bytes)',
  ].join('\n'));
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

test('updater artifacts and exact runtime updater config are required', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => {
    config.bundle.createUpdaterArtifacts = false;
  });
  await assert.rejects(verifyTauriGeneration(root), /createUpdaterArtifacts/);
  await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => {
    config.bundle.createUpdaterArtifacts = true;
    delete config.plugins;
  });
  await assert.rejects(verifyTauriGeneration(root), /runtime plugins/);
});

test('updater dependency must be exactly 2.13.1 in Cargo.toml and Cargo.lock', async (t) => {
  const root = await fixture(t);
  const manifest = join(root, 'apps/desktop/src-tauri/Cargo.toml');
  const original = await readFile(manifest, 'utf8');
  await writeFile(manifest, original.replace('tauri-plugin-updater = { version = "=2.13.1" }',
    'tauri-plugin-updater = { version = "2.13.1" }'));
  await assert.rejects(verifyTauriGeneration(root), /direct tauri-plugin-updater dependency is not exactly pinned/);
  await writeFile(manifest, original);
  const lock = join(root, 'Cargo.lock');
  await writeFile(lock, (await readFile(lock, 'utf8')).replace('name = "tauri-plugin-updater"\nversion = "2.13.1"',
    'name = "tauri-plugin-updater"\nversion = "2.13.0"'));
  await assert.rejects(verifyTauriGeneration(root), /locked Rust updater plugin/);
});

test('fixed endpoint, key and trust policy fail closed on every change', async (t) => {
  const changes = [
    ['endpoint', (u) => { u.endpoints = ['https://example.invalid/latest.json']; }, /Tauri updater endpoint/],
    ['multiple endpoints', (u) => { u.endpoints.push(UPDATER_ENDPOINT); }, /exactly one endpoint/],
    ['HTTP endpoint', (u) => { u.endpoints = ['http://example.invalid/latest.json']; }, /Tauri updater endpoint/],
    ['missing key', (u) => { delete u.pubkey; }, /reviewed fields/],
    ['wrong key', (u) => { u.pubkey = 'wrong'; }, /Tauri updater public key/],
    ['unsigned version', (u) => { u.requireSignedVersion = false; }, /Tauri requireSignedVersion/],
    ['downgrades', (u) => { u.allowDowngrades = true; }, /Tauri allowDowngrades/],
    ['insecure transport', (u) => { u.dangerousInsecureTransportProtocol = true; }, /Tauri dangerousInsecureTransportProtocol/],
    ['invalid certificates', (u) => { u.dangerousAcceptInvalidCerts = true; }, /Tauri dangerousAcceptInvalidCerts/],
    ['invalid hostnames', (u) => { u.dangerousAcceptInvalidHostnames = true; }, /Tauri dangerousAcceptInvalidHostnames/],
    ['extra updater field', (u) => { u.headers = { x: 'unsafe' }; }, /reviewed fields/],
  ];
  for (const [name, change, error] of changes) {
    await t.test(name, async (subtest) => {
      const root = await fixture(subtest);
      await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => change(config.plugins.updater));
      await assert.rejects(verifyTauriGeneration(root), error);
    });
  }
  await t.test('extra plugin', async (subtest) => {
    const root = await fixture(subtest);
    await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => {
      config.plugins.other = {};
    });
    await assert.rejects(verifyTauriGeneration(root), /runtime plugins/);
  });
  await t.test('changed committed public key', async (subtest) => {
    const root = await fixture(subtest);
    const key = Buffer.alloc(42, 1);
    key.write('Ed', 0, 'ascii');
    const keyId = Buffer.from(key.subarray(2, 10)).reverse().toString('hex').toUpperCase();
    const alternate = Buffer.from(
      `untrusted comment: minisign public key: ${keyId}\n${key.toString('base64')}\n`,
    ).toString('base64');
    await writeFile(join(root, PUBLIC_KEY_FILE), alternate);
    await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (config) => {
      config.plugins.updater.pubkey = alternate;
    });
    await assert.rejects(verifyTauriGeneration(root), /reviewed updater public key SHA-256/);
  });
});

test('renderer gets only the custom update command, with no direct updater capability', async (t) => {
  const root = await fixture(t);
  for (const permission of [
    'updater:default', 'updater:allow-check', 'updater:allow-download',
    'updater:allow-install', 'updater:allow-download-and-install',
  ]) {
    await changeJson(root, 'apps/desktop/src-tauri/capabilities/main.json', (capability) => {
      capability.permissions.push(permission);
    });
    await assert.rejects(verifyTauriGeneration(root), /direct updater plugin permissions/);
    await changeJson(root, 'apps/desktop/src-tauri/capabilities/main.json', (capability) => {
      capability.permissions.pop();
    });
  }
  for (const permission of [
    'allow-get-update-state',
    'allow-check-for-update',
    'allow-download-announced-update',
    'allow-install-verified-update',
  ]) {
    await changeJson(root, 'apps/desktop/src-tauri/capabilities/main.json', (capability) => {
      capability.permissions = capability.permissions.filter((candidate) => candidate !== permission);
    });
    await assert.rejects(verifyTauriGeneration(root), new RegExp(permission));
    await changeJson(root, 'apps/desktop/src-tauri/capabilities/main.json', (capability) => {
      capability.permissions.push(permission);
    });
  }
});

test('bounded downloader policy rejects resource and trust regressions', async (t) => {
  const root = await fixture(t);
  const file = join(root, 'apps/desktop/src-tauri/src/update_download.rs');
  const source = await readFile(file, 'utf8');
  await writeFile(file, source.replace('134_217_728', '268_435_456'));
  await assert.rejects(verifyTauriGeneration(root), /MAX_ARTIFACT_BYTES/);
  await writeFile(file, `${source}\nUpdate::download();\n`);
  await assert.rejects(verifyTauriGeneration(root), /Update::download/);
  await writeFile(file, source.replace('.retry(reqwest::retry::never())', ''));
  await assert.rejects(verifyTauriGeneration(root), /retry\(reqwest::retry::never/);
});

test('installer source is Windows-only with one retained native install call', async (t) => {
  const root = await fixture(t);
  const file = join(root, 'apps/desktop/src-tauri/src/update_install.rs');
  const source = await readFile(file, 'utf8');
  await writeFile(file, `${source}\nother.install(bytes);\n`);
  await assert.rejects(verifyTauriGeneration(root), /exactly one production \.install call/);
  await writeFile(file, source.replace('spawn_blocking', 'spawn'));
  await assert.rejects(verifyTauriGeneration(root), /spawn_blocking/);
  await writeFile(file, source);
  const updates = join(root, 'apps/desktop/src-tauri/src/updates.rs');
  const updateSource = await readFile(updates, 'utf8');
  await writeFile(
    updates,
    updateSource.replace('installation_supported() && verified_artifact_id.is_some()', 'true'),
  );
  await assert.rejects(verifyTauriGeneration(root), /installation_supported/);
});

test('JavaScript updater package is rejected in direct and locked npm dependencies', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'apps/desktop/package.json', (pkg) => {
    pkg.dependencies['@tauri-apps/plugin-updater'] = '2.13.1';
  });
  await assert.rejects(verifyTauriGeneration(root), /updater JavaScript package/);
  await changeJson(root, 'apps/desktop/package.json', (pkg) => {
    delete pkg.dependencies['@tauri-apps/plugin-updater'];
  });
  await changeJson(root, 'package-lock.json', (lock) => {
    lock.packages['node_modules/@tauri-apps/plugin-updater'] = { version: '2.13.1' };
  });
  await assert.rejects(verifyTauriGeneration(root), /updater JavaScript package/);
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

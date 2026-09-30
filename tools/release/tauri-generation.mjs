import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { REPOSITORY_ROOT, readJson } from './release-common.mjs';
import { verifyUpdaterPublicKey } from './updater-public-key.mjs';

export const TAURI_API_VERSION = '2.12.0';
export const TAURI_CLI_VERSION = '2.12.0';
export const TAURI_RUST_VERSION = '2.12.0';
export const TAURI_BUILD_VERSION = '2.7.0';
export const TAURI_UPDATER_VERSION = '2.13.1';
export const REQWEST_VERSION = '0.13.5';
export const MINISIGN_VERIFY_VERSION = '0.2.5';
export const BASE64_VERSION = '0.22.1';
export const SEMVER_VERSION = '1.0.28';
export const UPDATER_PUBLIC_KEY_SHA256 =
  '19215ba156d83fe9629e235dc06ab54ec6f9d30f07fd54c3c63adf62282615dd';
export const UPDATER_ENDPOINT =
  'https://github.com/unrealbg/NexusOps/releases/latest/download/latest.json';

function fail(message) {
  throw new Error(message);
}

function cargoDirectVersion(text, section, dependency) {
  let current = '';
  const found = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.startsWith('[')) {
      current = /^\[([^\]]+)\]$/.exec(line)?.[1] ?? '';
      continue;
    }
    if (current !== section || !line.startsWith(`${dependency} `)) continue;
    const match = new RegExp(
      `^${dependency}\\s*=\\s*\\{\\s*version\\s*=\\s*"=([0-9]+\\.[0-9]+\\.[0-9]+)"`,
    ).exec(line);
    if (!match) fail(`direct ${dependency} dependency is not exactly pinned`);
    found.push(match[1]);
  }
  if (found.length !== 1) fail(`direct ${dependency} dependency is missing or duplicated`);
  return found[0];
}

function cargoLockedVersion(text, name) {
  const matches = text.split(/^\[\[package\]\]\s*$/m).flatMap((block) => {
    const packageName = /^name = "([^"]+)"\s*$/m.exec(block)?.[1];
    const version = /^version = "([^"]+)"\s*$/m.exec(block)?.[1];
    return packageName === name ? [version] : [];
  });
  if (matches.length !== 1 || !matches[0])
    fail(`Cargo.lock ${name} package is missing or duplicated`);
  return matches[0];
}

function cargoLockedContainsVersion(text, name, expectedVersion) {
  const matches = text.split(/^\[\[package\]\]\s*$/m).filter((block) => {
    const packageName = /^name = "([^"]+)"\s*$/m.exec(block)?.[1];
    const version = /^version = "([^"]+)"\s*$/m.exec(block)?.[1];
    return packageName === name && version === expectedVersion;
  });
  if (matches.length !== 1)
    fail(`Cargo.lock ${name} ${expectedVersion} package is missing or duplicated`);
}

function expect(actual, expected, label) {
  if (actual !== expected) fail(`${label} must be exactly ${expected}`);
}

function expectExactKeys(value, expected, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value))
    fail(`${label} must be an object`);
  const actual = Object.keys(value).sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index]))
    fail(`${label} must have exactly the reviewed fields`);
}

function rejectUpdaterJsDependency(manifest, label) {
  for (const section of ['dependencies', 'devDependencies', 'optionalDependencies']) {
    if (Object.hasOwn(manifest?.[section] ?? {}, '@tauri-apps/plugin-updater'))
      fail(`${label} must not directly depend on the updater JavaScript package`);
  }
}

export async function verifyTauriGeneration(root = REPOSITORY_ROOT) {
  const tauriDirectory = join(root, 'apps/desktop/src-tauri');
  const [
    rootPackage,
    desktop,
    npmLock,
    rustManifest,
    cargoLock,
    config,
    capability,
    publicKey,
    downloadSource,
    updateSource,
  ] = await Promise.all([
    readJson(join(root, 'package.json'), 'root package.json'),
    readJson(join(root, 'apps/desktop/package.json'), 'desktop package.json'),
    readJson(join(root, 'package-lock.json'), 'package-lock.json'),
    readFile(join(root, 'apps/desktop/src-tauri/Cargo.toml'), 'utf8'),
    readFile(join(root, 'Cargo.lock'), 'utf8'),
    readJson(join(root, 'apps/desktop/src-tauri/tauri.conf.json'), 'tauri.conf.json'),
    readJson(join(root, 'apps/desktop/src-tauri/capabilities/main.json'), 'main capability'),
    verifyUpdaterPublicKey(root),
    readFile(join(root, 'apps/desktop/src-tauri/src/update_download.rs'), 'utf8'),
    readFile(join(root, 'apps/desktop/src-tauri/src/updates.rs'), 'utf8'),
  ]);
  rejectUpdaterJsDependency(rootPackage, 'root package.json');
  rejectUpdaterJsDependency(desktop, 'desktop package.json');
  rejectUpdaterJsDependency(npmLock.packages?.[''], 'locked root package');
  rejectUpdaterJsDependency(npmLock.packages?.['apps/desktop'], 'locked desktop package');
  if (Object.hasOwn(npmLock.packages ?? {}, 'node_modules/@tauri-apps/plugin-updater'))
    fail('updater JavaScript package must be absent from package-lock.json');
  expect(desktop.dependencies?.['@tauri-apps/api'], TAURI_API_VERSION, 'direct Tauri API');
  expect(desktop.devDependencies?.['@tauri-apps/cli'], TAURI_CLI_VERSION, 'direct Tauri CLI');
  expect(
    npmLock.packages?.['apps/desktop']?.dependencies?.['@tauri-apps/api'],
    TAURI_API_VERSION,
    'locked desktop Tauri API',
  );
  expect(
    npmLock.packages?.['apps/desktop']?.devDependencies?.['@tauri-apps/cli'],
    TAURI_CLI_VERSION,
    'locked desktop Tauri CLI',
  );
  expect(
    npmLock.packages?.['node_modules/@tauri-apps/api']?.version,
    TAURI_API_VERSION,
    'installed Tauri API',
  );
  expect(
    npmLock.packages?.['node_modules/@tauri-apps/cli']?.version,
    TAURI_CLI_VERSION,
    'installed Tauri CLI',
  );
  const nativeCliPackages = Object.entries(npmLock.packages ?? {}).filter(([name]) =>
    name.startsWith('node_modules/@tauri-apps/cli-'),
  );
  if (nativeCliPackages.length === 0) fail('native Tauri CLI lock entries are missing');
  for (const [name, value] of nativeCliPackages) {
    expect(value.version, TAURI_CLI_VERSION, name);
  }
  expect(
    cargoDirectVersion(rustManifest, 'dependencies', 'tauri'),
    TAURI_RUST_VERSION,
    'direct Rust Tauri',
  );
  expect(
    cargoDirectVersion(rustManifest, 'build-dependencies', 'tauri-build'),
    TAURI_BUILD_VERSION,
    'direct Rust tauri-build',
  );
  expect(cargoLockedVersion(cargoLock, 'tauri'), TAURI_RUST_VERSION, 'locked Rust Tauri');
  expect(
    cargoLockedVersion(cargoLock, 'tauri-build'),
    TAURI_BUILD_VERSION,
    'locked Rust tauri-build',
  );
  expect(
    cargoDirectVersion(rustManifest, 'dependencies', 'tauri-plugin-updater'),
    TAURI_UPDATER_VERSION,
    'direct Rust updater plugin',
  );
  expect(
    cargoLockedVersion(cargoLock, 'tauri-plugin-updater'),
    TAURI_UPDATER_VERSION,
    'locked Rust updater plugin',
  );
  for (const [name, version] of [
    ['reqwest', REQWEST_VERSION],
    ['minisign-verify', MINISIGN_VERIFY_VERSION],
    ['base64', BASE64_VERSION],
    ['semver', SEMVER_VERSION],
  ]) {
    expect(cargoDirectVersion(rustManifest, 'dependencies', name), version, `direct Rust ${name}`);
    cargoLockedContainsVersion(cargoLock, name, version);
  }
  if (config.bundle?.createUpdaterArtifacts !== true)
    fail('Tauri createUpdaterArtifacts must be true');
  expect(publicKey.sha256, UPDATER_PUBLIC_KEY_SHA256, 'reviewed updater public key SHA-256');
  expectExactKeys(config.plugins, ['updater'], 'Tauri runtime plugins');
  const updater = config.plugins.updater;
  expectExactKeys(
    updater,
    [
      'allowDowngrades',
      'dangerousAcceptInvalidCerts',
      'dangerousAcceptInvalidHostnames',
      'dangerousInsecureTransportProtocol',
      'endpoints',
      'pubkey',
      'requireSignedVersion',
    ],
    'Tauri updater configuration',
  );
  expect(updater.pubkey, publicKey.encodedPublicKey, 'Tauri updater public key');
  if (!Array.isArray(updater.endpoints) || updater.endpoints.length !== 1)
    fail('Tauri updater must have exactly one endpoint');
  expect(updater.endpoints[0], UPDATER_ENDPOINT, 'Tauri updater endpoint');
  expect(updater.requireSignedVersion, true, 'Tauri requireSignedVersion');
  expect(updater.allowDowngrades, false, 'Tauri allowDowngrades');
  expect(
    updater.dangerousInsecureTransportProtocol,
    false,
    'Tauri dangerousInsecureTransportProtocol',
  );
  expect(updater.dangerousAcceptInvalidCerts, false, 'Tauri dangerousAcceptInvalidCerts');
  expect(updater.dangerousAcceptInvalidHostnames, false, 'Tauri dangerousAcceptInvalidHostnames');
  if (
    !Array.isArray(capability.permissions) ||
    capability.permissions.some((permission) => typeof permission !== 'string')
  )
    fail('main capability permissions must be strings');
  for (const permission of [
    'allow-get-update-state',
    'allow-check-for-update',
    'allow-download-announced-update',
  ]) {
    if (capability.permissions.filter((candidate) => candidate === permission).length !== 1)
      fail(`main capability must grant exactly one custom ${permission} permission`);
  }
  if (capability.permissions.some((permission) => permission.startsWith('updater:')))
    fail('main capability must not grant direct updater plugin permissions');
  const requiredDownloadSource = [
    'MAX_ARTIFACT_BYTES: usize = 134_217_728',
    'CONNECT_TIMEOUT: Duration = Duration::from_secs(10)',
    'DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300)',
    'MAX_REDIRECTS: usize = 3',
    '"/unrealbg/NexusOps/releases/download/"',
    '"release-assets.githubusercontent.com"',
    '"objects.githubusercontent.com"',
    '.https_only(true)',
    '.no_proxy()',
  ];
  for (const fragment of requiredDownloadSource) {
    if (!downloadSource.includes(fragment)) fail(`bounded updater policy is missing ${fragment}`);
  }
  for (const forbidden of ['download_and_install', '.install(', 'Update::download']) {
    if (downloadSource.includes(forbidden) || updateSource.includes(forbidden))
      fail(`runtime updater source must not contain ${forbidden}`);
  }
  for (const forbiddenTls of [
    'danger_accept_invalid_certs(true)',
    'danger_accept_invalid_hostnames(true)',
    'dangerous_accept_invalid_certs(true)',
    'dangerous_accept_invalid_hostnames(true)',
  ]) {
    if (downloadSource.includes(forbiddenTls))
      fail(`bounded updater transport must not contain ${forbiddenTls}`);
  }
  if (config.build?.beforeBundleCommand)
    fail('Tauri beforeBundleCommand would run under signing credentials');
  const platformConfigs = (await readdir(tauriDirectory)).filter((name) =>
    /^tauri\.(?:windows|linux|macos)\.conf\.(?:json|json5|toml)$/.test(name),
  );
  if (platformConfigs.length > 0)
    fail('unreviewed platform-specific Tauri configuration is not allowed');
  return {
    api: TAURI_API_VERSION,
    cli: TAURI_CLI_VERSION,
    tauri: TAURI_RUST_VERSION,
    tauriBuild: TAURI_BUILD_VERSION,
  };
}

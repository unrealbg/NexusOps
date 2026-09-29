import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { REPOSITORY_ROOT, readJson } from './release-common.mjs';

export const TAURI_API_VERSION = '2.12.0';
export const TAURI_CLI_VERSION = '2.12.0';
export const TAURI_RUST_VERSION = '2.12.0';
export const TAURI_BUILD_VERSION = '2.7.0';

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

function expect(actual, expected, label) {
  if (actual !== expected) fail(`${label} must be exactly ${expected}`);
}

export async function verifyTauriGeneration(root = REPOSITORY_ROOT) {
  const tauriDirectory = join(root, 'apps/desktop/src-tauri');
  const [desktop, npmLock, rustManifest, cargoLock, config] = await Promise.all([
    readJson(join(root, 'apps/desktop/package.json'), 'desktop package.json'),
    readJson(join(root, 'package-lock.json'), 'package-lock.json'),
    readFile(join(root, 'apps/desktop/src-tauri/Cargo.toml'), 'utf8'),
    readFile(join(root, 'Cargo.lock'), 'utf8'),
    readJson(join(root, 'apps/desktop/src-tauri/tauri.conf.json'), 'tauri.conf.json'),
  ]);
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
  expect(cargoDirectVersion(rustManifest, 'dependencies', 'tauri'), TAURI_RUST_VERSION, 'direct Rust Tauri');
  expect(
    cargoDirectVersion(rustManifest, 'build-dependencies', 'tauri-build'),
    TAURI_BUILD_VERSION,
    'direct Rust tauri-build',
  );
  expect(cargoLockedVersion(cargoLock, 'tauri'), TAURI_RUST_VERSION, 'locked Rust Tauri');
  expect(cargoLockedVersion(cargoLock, 'tauri-build'), TAURI_BUILD_VERSION, 'locked Rust tauri-build');
  if (config.bundle?.createUpdaterArtifacts !== true)
    fail('Tauri createUpdaterArtifacts must be true');
  if (config.plugins && Object.keys(config.plugins).length > 0)
    fail('Tauri runtime plugin configuration is not allowed in Goal 04C');
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

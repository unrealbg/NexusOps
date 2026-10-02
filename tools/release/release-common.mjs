import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFile, lstat, realpath, readdir } from 'node:fs/promises';
import { basename, isAbsolute, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPOSITORY_ROOT = fileURLToPath(new URL('../../', import.meta.url));
export const PRODUCT_NAME = 'NexusOps';
export const MANIFEST_NAME = 'release-manifest.json';
export const MANIFEST_SCHEMA_VERSION = 1;

function fail(message) {
  throw new Error(message);
}

export function exactKeys(value, keys, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    fail(`${label} must be an object`);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    fail(`${label} has missing or unexpected fields`);
  }
}

export function assertSemver(value, label) {
  if (typeof value !== 'string') fail(`${label} must be a string`);
  const match =
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(
      value,
    );
  if (
    !match ||
    (match[4] &&
      match[4].split('.').some((part) => /^\d+$/.test(part) && !/^(0|[1-9]\d*)$/.test(part)))
  ) {
    fail(`${label} is not exact SemVer`);
  }
  return value;
}

export async function readJson(path, label) {
  let text;
  try {
    text = await readFile(path, 'utf8');
  } catch {
    fail(`${label} is missing or unreadable`);
  }
  try {
    return JSON.parse(text);
  } catch {
    fail(`${label} is malformed JSON`);
  }
}

export function workspacePackageVersion(toml) {
  let section = '';
  let version;
  for (const raw of toml.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    if (line.startsWith('[')) {
      const header = /^\[([^\]]+)\](?:\s*#.*)?$/.exec(line);
      if (!header) fail('Cargo workspace metadata has an invalid section header');
      section = header[1];
      continue;
    }
    if (section !== 'workspace.package' || !/^version\s*=/.test(line)) continue;
    if (version !== undefined) fail('Cargo workspace package version is duplicated');
    const assignment = /^version\s*=\s*"([^"\r\n]+)"(?:\s*#.*)?$/.exec(line);
    if (!assignment) fail('Cargo workspace package version must be a literal string');
    version = assignment[1];
  }
  if (version === undefined) fail('Cargo workspace package version is missing');
  return assertSemver(version, 'Cargo workspace package version');
}

function requiredVersion(object, key, label) {
  if (
    !object ||
    typeof object !== 'object' ||
    Array.isArray(object) ||
    !Object.hasOwn(object, key)
  ) {
    fail(`${label} is missing`);
  }
  return assertSemver(object[key], label);
}

export function cargoMetadata(root = REPOSITORY_ROOT) {
  let output;
  try {
    output = execFileSync(
      'cargo',
      ['metadata', '--no-deps', '--format-version', '1', '--locked', '--offline'],
      {
        cwd: root,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe'],
        maxBuffer: 16 * 1024 * 1024,
      },
    );
  } catch {
    fail('cargo metadata failed');
  }
  try {
    return JSON.parse(output);
  } catch {
    fail('cargo metadata returned malformed JSON');
  }
}

export async function verifyReleaseVersion(
  root = REPOSITORY_ROOT,
  metadataProvider = cargoMetadata,
) {
  const [rootPackage, desktopPackage, protocolPackage, uiPackage, tauri, lock, cargoToml] =
    await Promise.all([
    readJson(join(root, 'package.json'), 'root package.json'),
    readJson(join(root, 'apps/desktop/package.json'), 'desktop package.json'),
    readJson(join(root, 'packages/protocol/package.json'), 'protocol package.json'),
    readJson(join(root, 'packages/ui/package.json'), 'UI package.json'),
    readJson(join(root, 'apps/desktop/src-tauri/tauri.conf.json'), 'tauri.conf.json'),
    readJson(join(root, 'package-lock.json'), 'package-lock.json'),
    readFile(join(root, 'Cargo.toml'), 'utf8').catch(() =>
      fail('Cargo.toml is missing or unreadable'),
    ),
    ]);
  const metadata = await metadataProvider(root);
  if (!metadata || !Array.isArray(metadata.packages)) fail('cargo metadata has no packages');
  const desktopRust = metadata.packages.filter((entry) => entry?.name === 'nexus-desktop');
  if (desktopRust.length !== 1)
    fail('cargo metadata must contain exactly one nexus-desktop package');
  const versions = {
    'Cargo workspace': workspacePackageVersion(cargoToml),
    'nexus-desktop Cargo package': requiredVersion(
      desktopRust[0],
      'version',
      'nexus-desktop Cargo package version',
    ),
    'Tauri config': requiredVersion(tauri, 'version', 'Tauri config version'),
    'root package': requiredVersion(rootPackage, 'version', 'root package version'),
    'desktop package': requiredVersion(desktopPackage, 'version', 'desktop package version'),
    'protocol package': requiredVersion(protocolPackage, 'version', 'protocol package version'),
    'UI package': requiredVersion(uiPackage, 'version', 'UI package version'),
    'package-lock top level': requiredVersion(lock, 'version', 'package-lock top-level version'),
    'package-lock root': requiredVersion(
      lock.packages?.[''],
      'version',
      'package-lock root version',
    ),
    'package-lock desktop': requiredVersion(
      lock.packages?.['apps/desktop'],
      'version',
      'package-lock desktop version',
    ),
    'package-lock protocol': requiredVersion(
      lock.packages?.['packages/protocol'],
      'version',
      'package-lock protocol version',
    ),
    'package-lock UI': requiredVersion(
      lock.packages?.['packages/ui'],
      'version',
      'package-lock UI version',
    ),
  };
  const expected = versions['root package'];
  for (const [label, version] of Object.entries(versions)) {
    if (version !== expected) fail(`${label} does not match product version`);
  }
  return { productName: PRODUCT_NAME, productVersion: expected };
}

function git(root, args) {
  try {
    return execFileSync('git', args, {
      cwd: root,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
  } catch {
    fail(`git ${args[0]} failed`);
  }
}

export function verifyReleaseSource(root = REPOSITORY_ROOT, environment = process.env) {
  const sourceCommit = git(root, ['rev-parse', 'HEAD']);
  if (!/^[0-9a-f]{40}$/.test(sourceCommit)) fail('git HEAD is not a full commit SHA');
  if (environment.GITHUB_SHA !== undefined && environment.GITHUB_SHA !== sourceCommit) {
    fail('GITHUB_SHA does not match checked-out HEAD');
  }
  if (git(root, ['status', '--porcelain=v1', '--untracked-files=all']) !== '') {
    fail('release source working tree is dirty');
  }
  return sourceCommit;
}

export function detectPlatformArchitecture(platform = process.platform, rustcVerbose) {
  const platformName = { win32: 'windows', linux: 'linux', darwin: 'macos' }[platform];
  if (!platformName) fail('unsupported release platform');
  let output = rustcVerbose;
  if (output === undefined) {
    try {
      output = execFileSync('rustc', ['-vV'], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe'],
      });
    } catch {
      fail('rustc -vV failed');
    }
  }
  const host = /^host:\s*(\S+)\s*$/m.exec(output)?.[1];
  if (!host) fail('rustc host triple is missing');
  const architecture = host.split('-')[0];
  if (!['x86_64', 'aarch64'].includes(architecture)) fail('unsupported release architecture');
  const suffix = {
    windows: '-pc-windows-msvc',
    linux: '-unknown-linux-gnu',
    macos: '-apple-darwin',
  }[platformName];
  if (!host.endsWith(suffix)) fail('rustc host triple does not match supported release platform');
  return { platform: platformName, architecture };
}

export function expectedExecutable(platform) {
  if (platform === 'windows') return 'nexus-desktop.exe';
  if (platform === 'linux' || platform === 'macos') return 'nexus-desktop';
  fail('unsupported release platform');
}

function samePath(left, right) {
  return process.platform === 'win32' ? left.toLowerCase() === right.toLowerCase() : left === right;
}

export async function assertOrdinaryPath(path, type) {
  const absolute = resolve(path);
  const info = await lstat(absolute).catch(() => fail(`${type} is missing or unreadable`));
  if (info.isSymbolicLink() || (type === 'directory' ? !info.isDirectory() : !info.isFile())) {
    fail(`${type} is not an ordinary ${type}`);
  }
  const canonical = await realpath(absolute);
  if (!samePath(canonical, absolute)) fail(`${type} resolves through a link or reparse point`);
  return info;
}

export async function stagedFileNames(stage) {
  if (!isAbsolute(stage)) fail('staging directory must be absolute');
  await assertOrdinaryPath(stage, 'directory');
  const names = await readdir(stage);
  for (const name of names) {
    if (name === '.' || name === '..' || basename(name) !== name || name.includes(sep))
      fail('invalid staging entry');
    await assertOrdinaryPath(join(stage, name), 'file');
  }
  return names.sort();
}

export async function hashOrdinaryFile(path) {
  const before = await assertOrdinaryPath(path, 'file');
  const bytes = await readFile(path);
  const after = await assertOrdinaryPath(path, 'file');
  if (
    before.size !== after.size ||
    bytes.length !== before.size ||
    before.mtimeMs !== after.mtimeMs
  ) {
    fail('artifact changed while hashing');
  }
  return { bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
}

export function parseStageArgument(argv) {
  if (argv.length !== 2 || argv[0] !== '--stage' || !isAbsolute(argv[1])) {
    fail('expected exactly --stage <absolute-directory>');
  }
  return resolve(argv[1]);
}

export function releaseArtifactName({ productVersion, platform, architecture, sourceCommit }) {
  assertSemver(productVersion, 'product version');
  if (
    !['windows', 'linux', 'macos'].includes(platform) ||
    !['x86_64', 'aarch64'].includes(architecture)
  ) {
    fail('invalid release artifact platform or architecture');
  }
  if (!/^[0-9a-f]{40}$/.test(sourceCommit)) fail('invalid source commit');
  return `NexusOps-${productVersion}-${platform}-${architecture}-${sourceCommit.slice(0, 8)}`;
}

export function updaterCandidateArtifactName({
  productVersion,
  platform,
  architecture,
  sourceCommit,
}) {
  assertSemver(productVersion, 'product version');
  if (
    !['windows', 'linux', 'macos'].includes(platform) ||
    !['x86_64', 'aarch64'].includes(architecture)
  ) {
    fail('invalid updater candidate artifact platform or architecture');
  }
  if (!/^[0-9a-f]{40}$/.test(sourceCommit)) fail('invalid source commit');
  return `NexusOps-updater-${productVersion}-${platform}-${architecture}-${sourceCommit.slice(0, 8)}`;
}

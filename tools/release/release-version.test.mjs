import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import {
  detectPlatformArchitecture,
  verifyReleaseVersion,
  workspacePackageVersion,
} from './release-common.mjs';

const VERSION = '0.1.1';
const metadata = () => ({ packages: [{ name: 'nexus-desktop', version: VERSION }] });

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'nexusops-release-version-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'apps/desktop/src-tauri'), { recursive: true });
  await mkdir(join(root, 'packages/protocol'), { recursive: true });
  await mkdir(join(root, 'packages/ui'), { recursive: true });
  await writeFile(join(root, 'Cargo.toml'), `[workspace.package]\nversion = "${VERSION}"\n`);
  await writeFile(join(root, 'package.json'), JSON.stringify({ version: VERSION }));
  await writeFile(join(root, 'apps/desktop/package.json'), JSON.stringify({ version: VERSION }));
  await writeFile(join(root, 'packages/protocol/package.json'), JSON.stringify({ version: VERSION }));
  await writeFile(join(root, 'packages/ui/package.json'), JSON.stringify({ version: VERSION }));
  await writeFile(
    join(root, 'apps/desktop/src-tauri/tauri.conf.json'),
    JSON.stringify({ version: VERSION }),
  );
  await writeFile(
    join(root, 'package-lock.json'),
    JSON.stringify({
      version: VERSION,
      packages: {
        '': { version: VERSION },
        'apps/desktop': { version: VERSION },
        'packages/protocol': { version: VERSION },
        'packages/ui': { version: VERSION },
      },
    }),
  );
  return root;
}

async function changeJson(root, relative, change) {
  const path = join(root, relative);
  const value = JSON.parse(await readFile(path, 'utf8'));
  change(value);
  await writeFile(path, JSON.stringify(value));
}

test('all required product versions agree', async (t) => {
  const root = await fixture(t);
  assert.deepEqual(await verifyReleaseVersion(root, metadata), {
    productName: 'NexusOps',
    productVersion: VERSION,
  });
});

test('effective nexus-desktop Cargo version mismatch fails', async (t) => {
  const root = await fixture(t);
  await assert.rejects(
    verifyReleaseVersion(root, () => ({ packages: [{ name: 'nexus-desktop', version: '0.2.0' }] })),
    /nexus-desktop Cargo package.*does not match/,
  );
});

test('Cargo workspace version mismatch fails', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'Cargo.toml'), '[workspace.package]\nversion = "0.2.0"\n');
  await assert.rejects(verifyReleaseVersion(root, metadata), /Cargo workspace does not match/);
});

test('Tauri config mismatch fails', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'apps/desktop/src-tauri/tauri.conf.json', (data) => {
    data.version = '0.2.0';
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /Tauri config does not match/);
});

test('desktop package mismatch fails', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'apps/desktop/package.json', (data) => {
    data.version = '0.2.0';
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /desktop package does not match/);
});

for (const [label, path] of [
  ['protocol package', 'packages/protocol/package.json'],
  ['UI package', 'packages/ui/package.json'],
]) {
  test(`${label} mismatch fails`, async (t) => {
    const root = await fixture(t);
    await changeJson(root, path, (data) => {
      data.version = '0.2.0';
    });
    await assert.rejects(verifyReleaseVersion(root, metadata), new RegExp(`${label} does not match`));
  });
}

test('package-lock root or desktop mismatch fails', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'package-lock.json', (data) => {
    data.packages['apps/desktop'].version = '0.2.0';
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /package-lock desktop does not match/);
  await changeJson(root, 'package-lock.json', (data) => {
    data.packages['apps/desktop'].version = VERSION;
    data.packages[''].version = '0.2.0';
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /package-lock root does not match/);
});

test('package-lock protocol or UI mismatch fails', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'package-lock.json', (data) => {
    data.packages['packages/protocol'].version = '0.2.0';
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /package-lock protocol does not match/);
  await changeJson(root, 'package-lock.json', (data) => {
    data.packages['packages/protocol'].version = VERSION;
    data.packages['packages/ui'].version = '0.2.0';
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /package-lock UI does not match/);
});

test('invalid SemVer fails', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'package.json', (data) => {
    data.version = '01.0.0';
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /not exact SemVer/);
});

test('missing required field fails', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'apps/desktop/package.json', (data) => {
    delete data.version;
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /desktop package version is missing/);
});

test('wrong JSON type fails', async (t) => {
  const root = await fixture(t);
  await changeJson(root, 'package-lock.json', (data) => {
    data.packages[''].version = 100;
  });
  await assert.rejects(verifyReleaseVersion(root, metadata), /must be a string/);
});

test('malformed JSON fails', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'apps/desktop/package.json'), '{not-json');
  await assert.rejects(verifyReleaseVersion(root, metadata), /malformed JSON/);
});

test('workspace parser rejects duplicate and nonliteral versions', () => {
  assert.throws(
    () => workspacePackageVersion('[workspace.package]\nversion = "0.1.0"\nversion = "0.2.0"'),
    /duplicated/,
  );
  assert.throws(
    () => workspacePackageVersion('[workspace.package]\nversion = 1'),
    /literal string/,
  );
});

test('architecture comes from rustc host, not a runner label', () => {
  assert.deepEqual(detectPlatformArchitecture('win32', 'host: x86_64-pc-windows-msvc\n'), {
    platform: 'windows',
    architecture: 'x86_64',
  });
  assert.deepEqual(detectPlatformArchitecture('darwin', 'host: aarch64-apple-darwin\n'), {
    platform: 'macos',
    architecture: 'aarch64',
  });
  assert.throws(
    () => detectPlatformArchitecture('linux', 'host: armv7-unknown-linux-gnu\n'),
    /unsupported release architecture/,
  );
  assert.throws(
    () => detectPlatformArchitecture('linux', 'host: x86_64-apple-darwin\n'),
    /does not match/,
  );
});

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { REPOSITORY_ROOT } from './release-common.mjs';
import { verifyLifecycleSource, verifyLifecycleSourceText } from './lifecycle-source.mjs';

const desktop = (path) => join(REPOSITORY_ROOT, 'apps/desktop/src-tauri', path);

async function fixture() {
  const [commandsSource, mainSource, buildSource, capabilityText, lifecycleSource, updates, download, installation] =
    await Promise.all([
      readFile(desktop('src/commands.rs'), 'utf8'),
      readFile(desktop('src/main.rs'), 'utf8'),
      readFile(desktop('build.rs'), 'utf8'),
      readFile(desktop('capabilities/main.json'), 'utf8'),
      readFile(desktop('src/lifecycle.rs'), 'utf8'),
      readFile(desktop('src/updates.rs'), 'utf8'),
      readFile(desktop('src/update_download.rs'), 'utf8'),
      readFile(desktop('src/update_install.rs'), 'utf8'),
    ]);
  return {
    commandsSource,
    mainSource,
    buildSource,
    capability: JSON.parse(capabilityText),
    lifecycleSource,
    updaterSources: [updates, download],
    installationSource: installation,
  };
}

test('all custom commands retain one native admission permit and no renderer lifecycle control exists', async () => {
  const result = await verifyLifecycleSource(REPOSITORY_ROOT);
  assert.equal(result.commandCount, 53);
});

test('source policy fails closed when command admission is removed or released early', async () => {
  const value = await fixture();
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        commandsSource: value.commandsSource.replace(
          'let _permit = lifecycle.admit()?;',
          'let _permit = ();',
        ),
      }),
    /must acquire its lifecycle permit/,
  );
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        commandsSource: value.commandsSource.replace(
          'let _permit = lifecycle.admit()?;',
          'let _permit = lifecycle.admit()?; drop(_permit);',
        ),
      }),
    /complete invocation/,
  );
});

test('source policy rejects lifecycle renderer authority and unreviewed updater installation', async () => {
  const value = await fixture();
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        capability: {
          ...value.capability,
          permissions: [...value.capability.permissions, 'allow-seal-and-drain'],
        },
      }),
    /lifecycle-control permissions/,
  );
  assert.throws(
    () => verifyLifecycleSourceText({ ...value, updaterSources: ['update.install()'] }),
    /must not contain \.install/,
  );
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        installationSource: `${value.installationSource}\nupdate.install(bytes);`,
      }),
    /exactly one production \.install call/,
  );
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        updaterSources: value.updaterSources.map((source) =>
          source.replace('.restart_after_install(false)', ''),
        ),
      }),
    /restart_after_install/,
  );
});

test('source policy fixes the opaque install authority and exclusive cleanup sequence', async () => {
  const value = await fixture();
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        commandsSource: value.commandsSource.replace(
          'verified_artifact_id: VerifiedArtifactId,',
          'verified_artifact_id: VerifiedArtifactId, version: String,',
        ),
      }),
    /renderer authority/,
  );
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        commandsSource: value.commandsSource.replace('.seal_and_drain_others()', '.seal_and_drain()'),
      }),
    /install command order/,
  );
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        installationSource: value.installationSource.replace(
          'steps.revoke_local_grants()?;',
          'steps.finalize_logging()?;',
        ),
      }),
    /install cleanup order/,
  );
});

test('source policy requires the absolute drain bound and fixed cleanup order', async () => {
  const value = await fixture();
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        lifecycleSource: value.lifecycleSource.replace('Duration::from_secs(60)', 'Duration::MAX'),
      }),
    /Duration::from_secs\(60\)/,
  );
  assert.throws(
    () =>
      verifyLifecycleSourceText({
        ...value,
        lifecycleSource: value.lifecycleSource.replace(
          '.shutdown_updates()\n        .await',
          '.finalize_logging()\n        .into_future()\n        .await',
        ),
      }),
    /exit cleanup order/,
  );
});

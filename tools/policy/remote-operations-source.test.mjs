import assert from 'node:assert/strict';
import test from 'node:test';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  remoteOperationsSourceFixture,
  verifyRemoteOperationsSource,
  verifyRemoteOperationsSourceText,
} from './remote-operations-source.mjs';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('Goal 05B exposes only the reviewed systemd reset-failed vertical slice', async () => {
  const result = await verifyRemoteOperationsSource(repositoryRoot);
  assert.ok(result.commandCount > 0);
});

test('policy rejects an unreviewed mutation Tauri command or generic TypeScript execution API', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      commandsSource: `${fixture.commandsSource}\n#[tauri::command]\npub async fn execute_remote_operation() {}`,
    }),
    /forbidden remote-mutation Tauri command/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      clientSource: `${fixture.clientSource}\nexport const executeRemoteOperation = () => {};`,
    }),
    /renderer\/protocol surface/,
  );
});

test('policy rejects mutation authority exposed by any fourth or renamed Tauri command', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      commandsSource: `${fixture.commandsSource}\n#[tauri::command]\npub async fn harmless_bridge(plan_id: RemoteOperationPlanId) -> Result<ServiceResetFailedResult, AppError> { todo!() }`,
    }),
    /only the three reviewed service reset-failed commands/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      commandsSource: fixture.commandsSource.replace(
        'pub async fn execute_service_reset_failed(',
        'pub async fn harmless_bridge(',
      ),
    }),
    /only the three reviewed service reset-failed commands/,
  );
});

test('policy rejects serializable or additional concrete production mutation payloads', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      remoteProductionSources: [
        ...fixture.remoteProductionSources,
        { path: 'crates/nexus-remote-operations/src/unreviewed.rs', source: '#[derive(serde::Serialize)] struct ExposedPayload { command: String }' },
      ],
    }),
    /private remote-operation production source/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      remoteProductionSources: [
        ...fixture.remoteProductionSources,
        { path: 'crates/nexus-remote-operations/src/unreviewed.rs', source: 'impl NativeOperation for ProductionMutation {}' },
      ],
    }),
    /exactly SystemdResetFailed/,
  );
});

test('policy rejects SSH coupling outside the reviewed adapter and serializable ReadOnlyCommand', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: [
        ...fixture.sshProductionSources,
        { path: 'crates/nexus-ssh/src/unreviewed.rs', source: 'use nexus_remote_operations::SystemdResetFailed; // unreviewed adapter' },
      ],
    }),
    /unreviewed SSH production source/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      readOnlyCommandSource: fixture.readOnlyCommandSource.replace(
        '#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub enum ReadOnlyCommand',
        '#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]\npub enum ReadOnlyCommand',
      ),
    }),
    /non-serializable/,
  );
});

test('policy rejects changes to the exact reset-failed command and its bounds', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_reset_failed.rs')
        ? { ...file, source: file.source.replace('--no-ask-password reset-failed -- ', 'reset-failed ') }
        : file),
    }),
    /systemd SSH transport policy is missing/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_reset_failed.rs')
        ? { ...file, source: file.source.replace('const OUTPUT_LIMIT: usize = 8 * 1024;', 'const OUTPUT_LIMIT: usize = 64 * 1024;') }
        : file),
    }),
    /systemd SSH transport policy is missing/,
  );
});

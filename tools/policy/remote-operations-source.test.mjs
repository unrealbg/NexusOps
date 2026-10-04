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

test('Goal 05A exposes no production remote-mutation authority to renderer or SSH', async () => {
  const result = await verifyRemoteOperationsSource(repositoryRoot);
  assert.ok(result.commandCount > 0);
});

test('policy rejects a mutation Tauri command or TypeScript execution API', async () => {
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

test('policy rejects serializable or concrete production mutation payloads', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      remoteProductionSources: [
        ...fixture.remoteProductionSources,
        '#[derive(serde::Serialize)] struct ExposedPayload { command: String }',
      ],
    }),
    /private remote-operation production source/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      remoteProductionSources: [
        ...fixture.remoteProductionSources,
        'impl NativeOperation for ProductionMutation {}',
      ],
    }),
    /private remote-operation production source/,
  );
});

test('policy rejects SSH coupling and serializable ReadOnlyCommand', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshCargoSource: `${fixture.sshCargoSource}\nnexus-remote-operations.workspace = true`,
    }),
    /SSH production source/,
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

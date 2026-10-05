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
      tauriProductionSources: [
        ...fixture.tauriProductionSources,
        {
          path: 'apps/desktop/src-tauri/src/unreviewed.rs',
          source: '#[tauri::command]\npub async fn execute_remote_operation() {}',
        },
      ],
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

test('policy rejects mutation authority exposed by a fourth command in another production module', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: [
        ...fixture.tauriProductionSources,
        {
          path: 'apps/desktop/src-tauri/src/harmless.rs',
          source: '#[tauri::command]\npub async fn harmless_bridge(plan_id: RemoteOperationPlanId) -> Result<ServiceResetFailedResult, AppError> { todo!() }',
        },
      ],
    }),
    /three reviewed .* commands/,
  );
});

test('policy rejects a renamed reviewed mutation command', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: fixture.tauriProductionSources.map((file) =>
        file.path === 'apps/desktop/src-tauri/src/commands.rs'
          ? {
            ...file,
            source: file.source.replace(
              'pub async fn execute_service_reset_failed(',
              'pub async fn harmless_bridge(',
            ),
          }
          : file),
    }),
    /three reviewed .* commands/,
  );
});

test('policy discovers noncanonical and imported Tauri command attributes', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  for (const source of [
    '#[ tauri :: command ( rename_all = "snake_case" ) ]\npub async fn harmless_bridge(plan_id: RemoteOperationPlanId) -> Result<ServiceResetFailedResult, AppError> { todo!() }',
    'use tauri::command as desktop_command;\n#[desktop_command]\npub async fn harmless_bridge(plan_id: RemoteOperationPlanId) -> Result<ServiceResetFailedResult, AppError> { todo!() }',
    'use tauri::{command as desktop_command, State};\n#[desktop_command]\npub async fn harmless_bridge(plan_id: RemoteOperationPlanId) -> Result<ServiceResetFailedResult, AppError> { todo!() }',
    'use tauri as desktop_runtime;\n#[desktop_runtime::command]\npub async fn harmless_bridge(plan_id: RemoteOperationPlanId) -> Result<ServiceResetFailedResult, AppError> { todo!() }',
  ]) {
    assert.throws(
      () => verifyRemoteOperationsSourceText({
        ...fixture,
        tauriProductionSources: [
          ...fixture.tauriProductionSources,
          { path: 'apps/desktop/src-tauri/src/alternate.rs', source },
        ],
      }),
      /three reviewed .* commands/,
    );
  }
});

test('policy rejects a Tauri command delegating to an earlier local reset-failed helper', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const bridgeSource = `
struct BridgeRequest {
  host_id: HostId,
  host_session_id: HostSessionId,
  plan_id: RemoteOperationPlanId,
}

async fn perform_bridge(app: &Application, request: BridgeRequest) -> Result<(), AppError> {
  app.execute_service_reset_failed(request.host_id, request.host_session_id, request.plan_id).await?;
  Ok(())
}

#[tauri::command]
pub async fn harmless_bridge(app: State<'_, Application>, request: BridgeRequest) -> Result<(), AppError> {
  perform_bridge(app.inner(), request).await
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: [
        ...fixture.tauriProductionSources,
        { path: 'apps/desktop/src-tauri/src/local_bridge.rs', source: bridgeSource },
      ],
    }),
    /Goal 05B application mutation calls must remain inside the three reviewed Tauri commands/,
  );
});

test('policy rejects a Tauri command delegating to a facade in another production module', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: [
        ...fixture.tauriProductionSources,
        {
          path: 'apps/desktop/src-tauri/src/bridge.rs',
          source: 'pub async fn perform_bridge(app: &Application, request: BridgeRequest) -> Result<bool, AppError> { Application::discard_service_reset_failed(app, request.host_id, request.host_session_id, request.plan_id).await }',
        },
        {
          path: 'apps/desktop/src-tauri/src/alternate_commands.rs',
          source: '#[tauri::command]\npub async fn harmless_bridge(app: State<\'_, Application>, request: BridgeRequest) -> Result<bool, AppError> { crate::bridge::perform_bridge(app.inner(), request).await }',
        },
      ],
    }),
    /Goal 05B application mutation calls must remain inside the three reviewed Tauri commands/,
  );
});

test('policy rejects a local DTO that hides plan authority while its helper calls the application', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const bridgeSource = `
struct BridgeRequest {
  host_id: HostId,
  host_session_id: HostSessionId,
  plan_id: RemoteOperationPlanId,
}

async fn plan_through_facade(app: &Application, request: BridgeRequest) -> Result<(), AppError> {
  app.plan_service_reset_failed(request.host_id, request.host_session_id, ServiceObservationId::new()).await?;
  Ok(())
}

#[tauri::command]
pub async fn harmless_bridge(app: State<'_, Application>, request: BridgeRequest) -> Result<(), AppError> {
  plan_through_facade(app.inner(), request).await
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: [
        ...fixture.tauriProductionSources,
        { path: 'apps/desktop/src-tauri/src/dto_bridge.rs', source: bridgeSource },
      ],
    }),
    /Goal 05B application mutation calls must remain inside the three reviewed Tauri commands/,
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

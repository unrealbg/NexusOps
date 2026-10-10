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

test('Goal 05F exposes exactly the four reviewed operation-specific vertical slices', async () => {
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
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      protocolSource: `${fixture.protocolSource}\nexport type ServiceOperationPlan = { verb: string };`,
    }),
    /renderer\/protocol surface/,
  );
});

test('policy rejects renderer-supplied Start authority fields and multi-target command construction', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: fixture.tauriProductionSources.map((file) =>
        file.path === 'apps/desktop/src-tauri/src/commands.rs'
          ? {
            ...file,
            source: file.source.replace(
              'pub async fn plan_service_start(',
              'pub async fn plan_service_start(can_start: bool, ',
            ),
          }
          : file),
    }),
    /reviewed service mutation Tauri commands must not contain can_start: bool/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_start.rs')
        ? {
          ...file,
          source: file.source.replace(
            'format!("{COMMAND_PREFIX}{}", authority.target().as_str())',
            'format!("{COMMAND_PREFIX}{} {}", authority.target().as_str(), authority.target().as_str())',
          ),
        }
        : file),
    }),
    /must contain exactly the reviewed typed-authority exec path/,
  );
});

test('policy rejects mutation authority exposed by a thirteenth command in another production module', async () => {
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
    /twelve reviewed .* commands/,
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
    /twelve reviewed .* commands/,
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
      /twelve reviewed .* commands/,
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
    /Goal 05B\/05D\/05E\/05F application mutation references must remain inside the twelve reviewed Tauri commands/,
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
    /Goal 05B\/05D\/05E\/05F application mutation references must remain inside the twelve reviewed Tauri commands/,
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
    /Goal 05B\/05D\/05E\/05F application mutation references must remain inside the twelve reviewed Tauri commands/,
  );
});

test('policy rejects a reset-failed function item referenced through an Application type alias', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const bridgeSource = `
use nexus_core::Application as CoreApp;

pub async fn perform_bridge(app: &CoreApp, request: BridgeRequest) -> Result<(), AppError> {
  let execute = CoreApp::execute_service_reset_failed;
  execute(app, request.host_id, request.host_session_id, request.plan_id).await?;
  Ok(())
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: [
        ...fixture.tauriProductionSources,
        { path: 'apps/desktop/src-tauri/src/alias_bridge.rs', source: bridgeSource },
        {
          path: 'apps/desktop/src-tauri/src/alias_commands.rs',
          source: '#[tauri::command]\npub async fn harmless_bridge(app: State<\'_, Application>, request: BridgeRequest) -> Result<(), AppError> { crate::alias_bridge::perform_bridge(app.inner(), request).await }',
        },
      ],
    }),
    /Goal 05B\/05D\/05E\/05F application mutation references must remain inside the twelve reviewed Tauri commands/,
  );
});

test('policy rejects a reset-failed first-class function item reference', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const bridgeSource = `
pub async fn perform_bridge(app: &Application, request: BridgeRequest) -> Result<(), AppError> {
  let execute = Application::execute_service_reset_failed;
  execute(app, request.host_id, request.host_session_id, request.plan_id).await?;
  Ok(())
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: [
        ...fixture.tauriProductionSources,
        { path: 'apps/desktop/src-tauri/src/function_item_bridge.rs', source: bridgeSource },
        {
          path: 'apps/desktop/src-tauri/src/function_item_commands.rs',
          source: '#[tauri::command]\npub async fn harmless_bridge(app: State<\'_, Application>, request: BridgeRequest) -> Result<(), AppError> { crate::function_item_bridge::perform_bridge(app.inner(), request).await }',
        },
      ],
    }),
    /Goal 05B\/05D\/05E\/05F application mutation references must remain inside the twelve reviewed Tauri commands/,
  );
});

test('policy rejects a direct reset-failed UFCS call through an Application type alias', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const bridgeSource = `
use nexus_core::Application as CoreApp;

pub async fn perform_bridge(app: &CoreApp, request: BridgeRequest) -> Result<(), AppError> {
  CoreApp::execute_service_reset_failed(app, request.host_id, request.host_session_id, request.plan_id).await?;
  Ok(())
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: [
        ...fixture.tauriProductionSources,
        { path: 'apps/desktop/src-tauri/src/aliased_ufcs_bridge.rs', source: bridgeSource },
        {
          path: 'apps/desktop/src-tauri/src/aliased_ufcs_commands.rs',
          source: '#[tauri::command]\npub async fn harmless_bridge(app: State<\'_, Application>, request: BridgeRequest) -> Result<(), AppError> { crate::aliased_ufcs_bridge::perform_bridge(app.inner(), request).await }',
        },
      ],
    }),
    /Goal 05B\/05D\/05E\/05F application mutation references must remain inside the twelve reviewed Tauri commands/,
  );
});

test('policy rejects a public Application wrapper around reset-failed execution', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const source = `
impl Application {
  pub async fn harmless_reset_bridge(&self, request: BridgeRequest) -> Result<(), AppError> {
    self.execute_service_reset_failed(request.host_id, request.host_session_id, request.plan_id).await?;
    Ok(())
  }
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: [
        ...(fixture.coreProductionSources ?? []),
        { path: 'crates/nexus-core/src/application/harmless_bridge.rs', source },
      ],
    }),
    /Goal 05B nexus-core application boundary/,
  );
});

test('policy rejects a free nexus-core function delegating reset-failed execution', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const source = `
pub async fn harmless_reset_bridge(app: &Application, request: BridgeRequest) -> Result<(), AppError> {
  Application::execute_service_reset_failed(app, request.host_id, request.host_session_id, request.plan_id).await?;
  Ok(())
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: [
        ...(fixture.coreProductionSources ?? []),
        { path: 'crates/nexus-core/src/harmless_bridge.rs', source },
      ],
    }),
    /Goal 05B nexus-core application boundary/,
  );
});

test('policy rejects a public Application wrapper around reset-failed planning', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const source = `
impl Application {
  pub async fn harmless_plan_bridge(&self, request: BridgeRequest) -> Result<ServiceResetFailedPlan, AppError> {
    self.plan_service_reset_failed(request.host_id, request.host_session_id, request.observation_id).await
  }
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: [
        ...(fixture.coreProductionSources ?? []),
        { path: 'crates/nexus-core/src/application/harmless_plan.rs', source },
      ],
    }),
    /Goal 05B nexus-core application boundary/,
  );
});

test('policy rejects a public Application wrapper around reset-failed discard', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const source = `
impl Application {
  pub async fn harmless_discard_bridge(&self, request: BridgeRequest) -> Result<bool, AppError> {
    self.discard_service_reset_failed(request.host_id, request.host_session_id, request.plan_id).await
  }
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: [
        ...(fixture.coreProductionSources ?? []),
        { path: 'crates/nexus-core/src/application/harmless_discard.rs', source },
      ],
    }),
    /Goal 05B nexus-core application boundary/,
  );
});

test('policy rejects a nexus-core function-item alias of a reviewed Application method', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const source = `
use crate::Application as CoreApp;

pub async fn harmless_reset_bridge(app: &CoreApp, request: BridgeRequest) -> Result<(), AppError> {
  let execute = CoreApp::execute_service_reset_failed;
  execute(app, request.host_id, request.host_session_id, request.plan_id).await?;
  Ok(())
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: [
        ...(fixture.coreProductionSources ?? []),
        { path: 'crates/nexus-core/src/application/aliased_bridge.rs', source },
      ],
    }),
    /Goal 05B nexus-core application boundary/,
  );
});

test('policy rejects an Application wrapper using an aliased remote-operation foundation field', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const source = `
impl Application {
  pub async fn harmless_foundation_bridge(&self, request: BridgeRequest) -> Result<(), AppError> {
    let foundation = &self.remote_operations;
    foundation.execute::<SystemdResetFailed, _, _>(
      request.plan_id,
      request.binding,
      &request.revalidator,
      request.transport,
      request.cancellation,
    ).await?;
    Ok(())
  }
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: [
        ...(fixture.coreProductionSources ?? []),
        { path: 'crates/nexus-core/src/application/aliased_foundation.rs', source },
      ],
    }),
    /Goal 05B nexus-core remote-operation foundation access/,
  );
});

test('policy rejects an exported facade using an aliased RemoteOperationFoundation type', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const source = `
use nexus_remote_operations::RemoteOperationFoundation as Foundation;

pub async fn harmless_foundation_bridge(
  foundation: &Foundation,
  request: BridgeRequest,
) -> Result<(), AppError> {
  foundation.execute::<SystemdResetFailed, _, _>(
    request.plan_id,
    request.binding,
    &request.revalidator,
    request.transport,
    request.cancellation,
  ).await?;
  Ok(())
}`;
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: [
        ...(fixture.coreProductionSources ?? []),
        { path: 'crates/nexus-core/src/aliased_foundation.rs', source },
      ],
    }),
    /Goal 05B nexus-core remote-operation foundation access/,
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
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_reset_failed.rs')
        ? { ...file, source: file.source.replace(
          'ChannelMsg::ExitStatus { exit_status } =>',
          'ChannelMsg::ExitStatus { exit_status } if *accepted =>',
        ) }
        : file),
    }),
    /systemd SSH transport policy is missing|reviewed systemd SSH transport contains forbidden text/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_reset_failed.rs')
        ? { ...file, source: file.source.replace(
          'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
          'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close => MessageHandling::Continue,',
        ) }
        : file),
    }),
    /systemd SSH transport policy is missing|reviewed systemd SSH transport contains forbidden text/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_reset_failed.rs')
        ? { ...file, source: file.source.replace(
          'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
          'ChannelMsg::Eof => MessageHandling::Continue,',
        ) }
        : file),
    }),
    /systemd SSH transport policy is missing/,
  );
});

test('policy rejects changes to the exact try-restart semantic and transport bounds', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  for (const [from, to] of [
    ['--no-ask-password try-restart -- ', '--no-ask-password restart -- '],
    ['const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);', 'const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(60);'],
    ['const OUTPUT_LIMIT: usize = 8 * 1024;', 'const OUTPUT_LIMIT: usize = 16 * 1024;'],
    ['ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,', 'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close => MessageHandling::Continue,'],
  ]) {
    assert.throws(
      () => verifyRemoteOperationsSourceText({
        ...fixture,
        sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_try_restart.rs')
          ? { ...file, source: file.source.replace(from, to) }
          : file),
      }),
      /try-restart SSH transport policy is missing|reviewed try-restart SSH transport contains forbidden text/,
    );
  }
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      remoteProductionSources: fixture.remoteProductionSources.map((file) => file.path.endsWith('systemd_try_restart.rs')
        ? { ...file, source: file.source.replace('active == "active" && sub == "running"', 'active == "active"') }
        : file),
    }),
    /try-restart operation policy is missing/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: fixture.coreProductionSources.map((file) => file.path.endsWith('application/services.rs')
        ? { ...file, source: file.source.replace('.plan::<SystemdTryRestart>(', '.plan::<SystemdResetFailed>(') }
        : file),
    }),
    /native operation binding for plan_service_try_restart must remain fixed/,
  );
});

test('policy pins complete try-restart dispatch certainty and message classification', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const mutations = [
    [
      'ChannelMsg::ExitStatus { exit_status } =>',
      'ChannelMsg::ExitStatus { exit_status } if *accepted =>',
    ],
    [
      'ChannelMsg::ExitSignal { .. } =>',
      'ChannelMsg::ExitSignal { .. } if *accepted =>',
    ],
    [
      'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
      'ChannelMsg::Eof => MessageHandling::Continue,',
    ],
    [
      'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
      'ChannelMsg::Eof => MessageHandling::Continue,\n        ChannelMsg::WindowAdjusted { .. } => MessageHandling::Complete(MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::ConnectionLost)),',
    ],
    [
      'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
      'ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,\n        ChannelMsg::Eof => MessageHandling::Complete(MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::ConnectionLost)),',
    ],
    [
      'ChannelMsg::Failure if !*accepted && !*execution_evidence =>',
      'ChannelMsg::Failure =>',
    ],
    [
      'ChannelMsg::Failure => MessageHandling::Complete(',
      'ChannelMsg::Failure => MessageHandling::Continue /*',
    ],
    [
      'ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } =>',
      'ChannelMsg::ExtendedData { data, .. } =>',
    ],
    [
      'ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } =>',
      'ChannelMsg::Data { data } =>',
    ],
    ['*execution_evidence = true;', 'let _ = execution_evidence;'],
    [
      '*output_bytes = output_bytes.saturating_add(data.len());',
      'let _ = data;',
    ],
    ['if *output_bytes > OUTPUT_LIMIT', 'if false'],
    [
      `_ if !*accepted => MessageHandling::Complete(MutationTransportOutcome::CompletionUnknown(
            CompletionUnknownReason::ConnectionLost,
        )),
        _ => MessageHandling::Continue,`,
      '_ => MessageHandling::Continue,',
    ],
  ];

  for (const [from, to] of mutations) {
    assert.throws(
      () => verifyRemoteOperationsSourceText({
        ...fixture,
        sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_try_restart.rs')
          ? { ...file, source: file.source.replace(from, to) }
          : file),
      }),
      /try-restart SSH transport policy is missing|reviewed try-restart SSH transport contains forbidden text/,
    );
  }
});

test('policy rejects generic arbitrary-command helpers inside reviewed SSH adapters', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const genericHelper = `
async fn dispatch_arbitrary<C>(
    channel: &mut C,
    remote_command: &str,
) {
    let _ = channel.exec(true, remote_command).await;
}

`;

  for (const adapter of ['systemd_reset_failed.rs', 'systemd_try_restart.rs', 'systemd_reload.rs', 'systemd_start.rs']) {
    assert.throws(
      () => verifyRemoteOperationsSourceText({
        ...fixture,
        sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith(adapter)
          ? {
            ...file,
            source: file.source.replace(
              '#[cfg(test)]',
              `${genericHelper}#[cfg(test)]`,
            ),
          }
          : file),
      }),
      /must contain exactly the reviewed typed-authority exec path|must not contain command: &str/,
    );
  }
});

test('policy pins the reload operation, command, bounds and direct terminal truth', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const mutations = [
    ['--no-ask-password reload -- ', '--no-ask-password reload-or-restart -- '],
    ['const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);', 'const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(60);'],
    ['ChannelMsg::ExitStatus { exit_status } =>', 'ChannelMsg::ExitStatus { exit_status } if *accepted =>'],
    ['ChannelMsg::ExitSignal { .. } =>', 'ChannelMsg::ExitSignal { .. } if *accepted =>'],
    ['ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,', 'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close => MessageHandling::Continue,'],
    ['ChannelMsg::Failure if !*accepted && !*execution_evidence =>', 'ChannelMsg::Failure =>'],
    ['*output_bytes = output_bytes.saturating_add(data.len());', '*output_bytes += data.len();'],
  ];
  for (const [from, to] of mutations) {
    assert.throws(
      () => verifyRemoteOperationsSourceText({
        ...fixture,
        sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_reload.rs')
          ? { ...file, source: file.source.replace(from, to) }
          : file),
      }),
      /reload SSH transport policy is missing|reviewed reload SSH transport contains forbidden text/,
    );
  }
});

test('policy pins the start operation, command, bounds and direct terminal truth', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  const mutations = [
    ['--no-ask-password start -- ', '--no-ask-password restart -- '],
    ['const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);', 'const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(60);'],
    ['ChannelMsg::ExitStatus { exit_status } =>', 'ChannelMsg::ExitStatus { exit_status } if *accepted =>'],
    ['ChannelMsg::ExitSignal { .. } =>', 'ChannelMsg::ExitSignal { .. } if *accepted =>'],
    ['ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,', 'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close => MessageHandling::Continue,'],
    ['ChannelMsg::Failure if !*accepted && !*execution_evidence =>', 'ChannelMsg::Failure =>'],
    ['*output_bytes = output_bytes.saturating_add(data.len());', '*output_bytes += data.len();'],
  ];
  for (const [from, to] of mutations) {
    assert.throws(
      () => verifyRemoteOperationsSourceText({
        ...fixture,
        sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_start.rs')
          ? { ...file, source: file.source.replace(from, to) }
          : file),
      }),
      /start SSH transport policy is missing|reviewed start SSH transport contains forbidden text/,
    );
  }
});

test('policy rejects a fifth native operation, a thirteenth mutation command and operation leakage outside adapters', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      remoteProductionSources: [...fixture.remoteProductionSources, {
        path: 'crates/nexus-remote-operations/src/fifth.rs',
        source: 'pub struct Fifth; impl NativeOperation for Fifth { type Target = (); type Preconditions = (); type Payload = (); const RISK: OperationRisk = OperationRisk::High; }',
      }],
    }),
    /exactly SystemdResetFailed, SystemdTryRestart, SystemdReload and SystemdStart/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      tauriProductionSources: [...fixture.tauriProductionSources, {
        path: 'apps/desktop/src-tauri/src/thirteenth.rs',
        source: '#[tauri::command]\npub async fn execute_service_fifth() {}',
      }],
    }),
    /forbidden remote-mutation Tauri command/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('systemd_try_restart.rs')
        ? { ...file, source: file.source.replace('try-restart -- ', 'reload -- ') }
        : file),
    }),
    /try-restart SSH transport policy is missing|reviewed try-restart SSH transport contains forbidden text/,
  );
});

test('policy pins the exact CanStart/CanReload inventory command and operation bindings', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      readOnlyCommandSource: fixture.readOnlyCommandSource.replace(
        ' --property=CanReload --property=Description show',
        ' --property=Description show',
      ),
    }),
    /service inventory command must remain fixed/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: fixture.coreProductionSources.map((file) => file.path.endsWith('application/services.rs')
        ? { ...file, source: file.source.replace('.plan::<SystemdReload>(', '.plan::<SystemdTryRestart>(') }
        : file),
    }),
    /native operation binding for plan_service_reload must remain fixed/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      readOnlyCommandSource: fixture.readOnlyCommandSource.replace(
        ' --property=CanStart --property=CanReload',
        ' --property=CanReload',
      ),
    }),
    /service inventory command must remain fixed/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      coreProductionSources: fixture.coreProductionSources.map((file) => file.path.endsWith('application/services.rs')
        ? { ...file, source: file.source.replace('.plan::<SystemdStart>(', '.plan::<SystemdReload>(') }
        : file),
    }),
    /native operation binding for plan_service_start must remain fixed/,
  );
});

test('policy pins the Goal 05G fixed query, 37 properties and traversal bounds', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      stopImpactQuerySource: fixture.stopImpactQuerySource.replace('"FailureAction",', '"CollectMode",'),
    }),
    /fixed 37-property systemd query contract changed/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      stopImpactQuerySource: fixture.stopImpactQuerySource.replace('--no-ask-password --all', '--all'),
    }),
    /fixed native read-only query is missing/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      stopImpactDiscoverySource: fixture.stopImpactDiscoverySource.replace(
        'const MAX_QUERIES: u8 = 6;',
        'const MAX_QUERIES: u8 = 7;',
      ),
    }),
    /bounded graph implementation is missing/,
  );
  for (const fragment of [
    'fn match_batch_by_identity(',
    'fn has_direct_frontier(&self) -> bool',
    'StopImpactConditionalClassification::CoverageUnknown',
    'std::num::NonZeroU32::new',
  ]) {
    assert.throws(
      () => verifyRemoteOperationsSourceText({
        ...fixture,
        stopImpactDiscoverySource: fixture.stopImpactDiscoverySource.replaceAll(fragment, 'removed_policy_fragment'),
      }),
      /bounded graph implementation is missing/,
    );
  }
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      sshProductionSources: fixture.sshProductionSources.map((file) => file.path.endsWith('session.rs')
        ? { ...file, source: file.source.replace('collect_stop_impact_output(&mut reader)', 'collect_output(&mut reader)') }
        : file),
    }),
    /SSH transport must remain closed over typed read-only commands/,
  );
});

test('policy rejects Goal 05G renderer targets and inspection-to-mutation bridges', async () => {
  const fixture = await remoteOperationsSourceFixture(repositoryRoot);
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      clientSource: fixture.clientSource.replace(
        'inspectionId: SystemdStopImpactInspectionId,',
        'inspectionId: SystemdStopImpactInspectionId, unit: string,',
      ),
    }),
    /renderer request must carry only opaque inspection authority/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      remoteProductionSources: [...fixture.remoteProductionSources, {
        path: 'crates/nexus-remote-operations/src/stop_bridge.rs',
        source: 'pub fn bridge(value: SystemdStopImpactInspectionId) { let _ = value; }',
      }],
    }),
    /inspection identity or result must not enter mutation authority/,
  );
  assert.throws(
    () => verifyRemoteOperationsSourceText({
      ...fixture,
      stopImpactQuerySource: fixture.stopImpactQuerySource.replace(
        'const QUERY_TIMEOUT: Duration = Duration::from_secs(8);',
        'const QUERY_TIMEOUT: Duration = Duration::from_secs(8);\nfn retry() {}',
      ),
    }),
    /contains forbidden fragment retry/,
  );
});

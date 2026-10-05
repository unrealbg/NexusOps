import { readdir, readFile } from 'node:fs/promises';
import { join, relative } from 'node:path';

function fail(message) {
  throw new Error(message);
}

function requireAbsent(text, fragments, context) {
  for (const fragment of fragments) {
    if (text.includes(fragment)) fail(`${context} must not contain ${fragment}`);
  }
}

export function verifyRemoteOperationsSourceText({
  commandsSource,
  protocolSource,
  clientSource,
  remoteProductionSources,
  sshProductionSources,
  sshCargoSource,
  readOnlyCommandSource,
  capabilitySource,
}) {
  const commandNames = [...commandsSource.matchAll(/#\[tauri::command\][\s\S]*?pub\s+(?:async\s+)?fn\s+([a-z0-9_]+)/g)]
    .map((match) => match[1]);
  const commandSections = commandsSource.split('#[tauri::command]').slice(1).map((source) => ({
    name: /pub\s+(?:async\s+)?fn\s+([a-z0-9_]+)/.exec(source)?.[1],
    source,
  }));
  const allowedMutationCommands = new Set([
    'plan_service_reset_failed',
    'discard_service_reset_failed',
    'execute_service_reset_failed',
  ]);
  const forbiddenCommands = new Set([
    'plan_remote_operation',
    'execute_remote_operation',
    'discard_remote_operation',
    'restart_service',
    'stop_service',
    'start_service',
    'reset_failed',
    'run_command',
    'execute_command',
    'shell',
    'sudo',
  ]);
  for (const name of commandNames) {
    if (forbiddenCommands.has(name)
      || (name.startsWith('plan_service_') && !allowedMutationCommands.has(name))
      || (name.startsWith('execute_service_') && !allowedMutationCommands.has(name))
      || (name.startsWith('discard_service_') && !allowedMutationCommands.has(name))
      || name.startsWith('docker_')) {
      fail(`forbidden remote-mutation Tauri command: ${name}`);
    }
  }
  const mutationSurfaceFragments = [
    'ServiceObservationId',
    'RemoteOperationPlanId',
    'ServiceResetFailedPlan',
    'ServiceResetFailedResult',
    'ServiceResetFailedOutcome',
    'ServiceResetFailedAuditStatus',
    'ServiceResetFailedPostObservationStatus',
    '.plan_service_reset_failed(',
    '.discard_service_reset_failed(',
    '.execute_service_reset_failed(',
  ];
  const mutationSurfaceCommands = commandSections.filter(({ source }) =>
    mutationSurfaceFragments.some((fragment) => source.includes(fragment)));
  if (mutationSurfaceCommands.length !== 3
      || mutationSurfaceCommands.some(({ name }) => !allowedMutationCommands.has(name))
      || [...allowedMutationCommands].some((allowed) =>
        mutationSurfaceCommands.filter(({ name }) => name === allowed).length !== 1)) {
    fail('only the three reviewed service reset-failed commands may expose mutation authority');
  }

  const rendererSurface = `${protocolSource}\n${clientSource}`;
  requireAbsent(
    rendererSurface,
    [
      'RemoteOperationOutcome',
      'planRemoteOperation',
      'executeRemoteOperation',
      'discardRemoteOperation',
      'plan_remote_operation',
      'execute_remote_operation',
      'discard_remote_operation',
    ],
    'renderer/protocol surface',
  );

  const remoteProduction = remoteProductionSources.map(({ source }) => source).join('\n');
  const systemdOperation = remoteProductionSources.find(({ path }) =>
    path === 'crates/nexus-remote-operations/src/systemd_reset_failed.rs');
  if (!systemdOperation) fail('reviewed SystemdResetFailed operation module is missing');
  const systemdOperationSource = systemdOperation.source;
  const systemdOperationProduction = systemdOperationSource.split('#[cfg(test)]')[0];
  requireAbsent(
    remoteProduction,
    [
      'serde::Serialize',
      'serde::Deserialize',
      'ts_rs::TS',
      '#[derive(Serialize',
      '#[derive(Deserialize',
      'command: String',
      'executable: String',
      'argv: Vec<String>',
      'environment: ',
    ],
    'private remote-operation production source',
  );
  for (const required of [
    'pub const PLAN_TTL: Duration = Duration::from_secs(120);',
    'RemoteOperationOutcome::OutcomeUnknown',
    'CompletionUnknownReason',
    'try_acquire_owned()',
  ]) {
    if (!remoteProduction.includes(required)) fail(`remote-operation policy is missing ${required}`);
  }
  for (const permission of [
    'allow-plan-service-reset-failed',
    'allow-discard-service-reset-failed',
    'allow-execute-service-reset-failed',
  ]) {
    if (!capabilitySource.includes(`"${permission}"`)) {
      fail(`reviewed reset-failed capability is missing ${permission}`);
    }
  }

  const nativeImplementations = remoteProduction.match(/impl NativeOperation for/g) ?? [];
  if (nativeImplementations.length !== 1
      || !systemdOperationProduction.includes('impl NativeOperation for SystemdResetFailed')) {
    fail('private remote-operation production source must contain exactly SystemdResetFailed');
  }
  for (const required of [
    'SystemdResetFailedPreconditions',
    'const RISK: OperationRisk = OperationRisk::Moderate;',
    'const MAX_UNIT_BYTES: usize = 255;',
  ]) {
    if (!systemdOperationProduction.includes(required)) fail(`systemd operation policy is missing ${required}`);
  }

  if (!sshCargoSource.includes('nexus-remote-operations = { path = "../nexus-remote-operations" }')) {
    fail('SSH production source is missing the reviewed remote-operation dependency');
  }
  const systemdTransport = sshProductionSources.find(({ path }) =>
    path === 'crates/nexus-ssh/src/systemd_reset_failed.rs');
  if (!systemdTransport) fail('reviewed SystemdResetFailed SSH adapter is missing');
  const systemdTransportSource = systemdTransport.source;
  const systemdTransportProduction = systemdTransportSource.split('#[cfg(test)]')[0];
  const otherSshSources = sshProductionSources
    .filter(({ path }) => path !== systemdTransport.path)
    .map(({ source }) => source)
    .join('\n');
  requireAbsent(
    otherSshSources,
    ['nexus_remote_operations', 'impl MutationTransport<'],
    'unreviewed SSH production source',
  );
  for (const required of [
    'impl MutationTransport<SystemdResetFailed> for SshSession',
    'const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);',
    'const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(15);',
    'const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);',
    'const OUTPUT_LIMIT: usize = 8 * 1024;',
    'LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password reset-failed -- ',
    'Some(ChannelMsg::ExitStatus { exit_status }) if accepted =>',
    'Some(ChannelMsg::ExitSignal { .. }) if accepted =>',
    'Some(ChannelMsg::Success) if !accepted => accepted = true,',
  ]) {
    if (!systemdTransportProduction.includes(required)) fail(`systemd SSH transport policy is missing ${required}`);
  }
  requireAbsent(
    systemdTransportProduction,
    ['sudo', 'pkexec', 'Vec<SystemdServiceUnitName>', 'command: String', 'verb: String'],
    'reviewed systemd SSH transport',
  );
  if (!systemdTransportProduction.includes('authority.target().as_str()')) {
    fail('systemd SSH transport must append exactly one typed authority target');
  }
  for (const required of [
    'plan_service_reset_failed',
    'discard_service_reset_failed',
    'execute_service_reset_failed',
    'ServiceResetFailedPlan',
    'ServiceResetFailedResult',
  ]) {
    if (!rendererSurface.includes(required) && !commandsSource.includes(required)) {
      fail(`reviewed reset-failed surface is missing ${required}`);
    }
  }

  const readOnlyDeclaration = /#\[derive\(([^)]*)\)\]\s*pub enum ReadOnlyCommand/.exec(
    readOnlyCommandSource,
  );
  if (!readOnlyDeclaration) fail('ReadOnlyCommand declaration is missing');
  if (/\b(?:Serialize|Deserialize|TS)\b/.test(readOnlyDeclaration[1])) {
    fail('ReadOnlyCommand must remain native and non-serializable');
  }

  return { commandCount: commandNames.length };
}

async function rustSources(root, directory, excluded = new Set()) {
  const entries = await readdir(directory, { withFileTypes: true });
  const values = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) values.push(...await rustSources(root, path, excluded));
    else if (entry.name.endsWith('.rs') && !excluded.has(entry.name)) values.push({
      path: relative(root, path).replaceAll('\\', '/'),
      source: await readFile(path, 'utf8'),
    });
  }
  return values;
}

export async function remoteOperationsSourceFixture(root) {
  return {
    commandsSource: await readFile(join(root, 'apps/desktop/src-tauri/src/commands.rs'), 'utf8'),
    protocolSource: await readFile(join(root, 'packages/protocol/src/index.ts'), 'utf8'),
    clientSource: await readFile(join(root, 'apps/desktop/src/api/client.ts'), 'utf8'),
    remoteProductionSources: await rustSources(
      root,
      join(root, 'crates/nexus-remote-operations/src'),
      new Set(['tests.rs']),
    ),
    sshProductionSources: await rustSources(root, join(root, 'crates/nexus-ssh/src')),
    sshCargoSource: await readFile(join(root, 'crates/nexus-ssh/Cargo.toml'), 'utf8'),
    readOnlyCommandSource: await readFile(join(root, 'crates/nexus-operations/src/command.rs'), 'utf8'),
    capabilitySource: await readFile(join(root, 'apps/desktop/src-tauri/capabilities/main.json'), 'utf8'),
  };
}

export async function verifyRemoteOperationsSource(root) {
  return verifyRemoteOperationsSourceText(await remoteOperationsSourceFixture(root));
}

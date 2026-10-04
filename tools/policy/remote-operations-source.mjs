import { readdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';

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
}) {
  const commandNames = [...commandsSource.matchAll(/#\[tauri::command\][\s\S]*?pub\s+(?:async\s+)?fn\s+([a-z0-9_]+)/g)]
    .map((match) => match[1]);
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
    if (forbiddenCommands.has(name) || name.startsWith('plan_service_') || name.startsWith('docker_')) {
      fail(`forbidden remote-mutation Tauri command: ${name}`);
    }
  }

  const rendererSurface = `${protocolSource}\n${clientSource}`;
  requireAbsent(
    rendererSurface,
    [
      'RemoteOperationPlan',
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

  const remoteProduction = remoteProductionSources.join('\n');
  requireAbsent(
    remoteProduction,
    [
      'serde::Serialize',
      'serde::Deserialize',
      'ts_rs::TS',
      '#[derive(Serialize',
      '#[derive(Deserialize',
      'impl NativeOperation for',
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

  requireAbsent(
    `${sshCargoSource}\n${sshProductionSources.join('\n')}`,
    ['nexus-remote-operations', 'nexus_remote_operations'],
    'SSH production source',
  );

  const readOnlyDeclaration = /#\[derive\(([^)]*)\)\]\s*pub enum ReadOnlyCommand/.exec(
    readOnlyCommandSource,
  );
  if (!readOnlyDeclaration) fail('ReadOnlyCommand declaration is missing');
  if (/\b(?:Serialize|Deserialize|TS)\b/.test(readOnlyDeclaration[1])) {
    fail('ReadOnlyCommand must remain native and non-serializable');
  }

  return { commandCount: commandNames.length };
}

async function rustSources(directory, excluded = new Set()) {
  const entries = await readdir(directory, { withFileTypes: true });
  const values = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) values.push(...await rustSources(path, excluded));
    else if (entry.name.endsWith('.rs') && !excluded.has(entry.name)) values.push(await readFile(path, 'utf8'));
  }
  return values;
}

export async function remoteOperationsSourceFixture(root) {
  return {
    commandsSource: await readFile(join(root, 'apps/desktop/src-tauri/src/commands.rs'), 'utf8'),
    protocolSource: await readFile(join(root, 'packages/protocol/src/index.ts'), 'utf8'),
    clientSource: await readFile(join(root, 'apps/desktop/src/api/client.ts'), 'utf8'),
    remoteProductionSources: await rustSources(
      join(root, 'crates/nexus-remote-operations/src'),
      new Set(['tests.rs']),
    ),
    sshProductionSources: await rustSources(join(root, 'crates/nexus-ssh/src')),
    sshCargoSource: await readFile(join(root, 'crates/nexus-ssh/Cargo.toml'), 'utf8'),
    readOnlyCommandSource: await readFile(join(root, 'crates/nexus-operations/src/command.rs'), 'utf8'),
  };
}

export async function verifyRemoteOperationsSource(root) {
  return verifyRemoteOperationsSourceText(await remoteOperationsSourceFixture(root));
}

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

function rustFunctionEnd(source, start, limit, context) {
  let bodyStart = -1;
  let depth = 0;
  for (let index = start; index < limit; index += 1) {
    if (source.startsWith('//', index)) {
      const newline = source.indexOf('\n', index + 2);
      index = newline === -1 ? limit : newline;
      continue;
    }
    if (source.startsWith('/*', index)) {
      let commentDepth = 1;
      index += 2;
      while (index < limit && commentDepth > 0) {
        if (source.startsWith('/*', index)) {
          commentDepth += 1;
          index += 2;
        } else if (source.startsWith('*/', index)) {
          commentDepth -= 1;
          index += 2;
        } else {
          index += 1;
        }
      }
      index -= 1;
      continue;
    }

    const rawString = /^(?:b)?r(#{0,255})"/.exec(source.slice(index));
    if (rawString) {
      const terminator = `"${rawString[1]}`;
      const end = source.indexOf(terminator, index + rawString[0].length);
      if (end === -1 || end >= limit) fail(`unterminated raw string in ${context}`);
      index = end + terminator.length - 1;
      continue;
    }
    const quoteOffset = source.startsWith('b"', index) ? 1 : 0;
    if (source[index + quoteOffset] === '"') {
      index += quoteOffset + 1;
      while (index < limit) {
        if (source[index] === '\\') index += 2;
        else if (source[index] === '"') break;
        else index += 1;
      }
      if (index >= limit) fail(`unterminated string in ${context}`);
      continue;
    }
    const character = /^'(?:\\.|[^'\\\r\n])'/.exec(source.slice(index));
    if (character) {
      index += character[0].length - 1;
      continue;
    }

    if (source[index] === '{') {
      if (bodyStart === -1) bodyStart = index;
      depth += 1;
    } else if (source[index] === '}' && bodyStart !== -1) {
      depth -= 1;
      if (depth === 0) return index + 1;
    }
  }
  fail(`Tauri command function body is missing or unterminated in ${context}`);
}

function rustIdentifierIndices(source, identifier, context) {
  const indices = [];
  for (let index = 0; index < source.length; index += 1) {
    if (source.startsWith('//', index)) {
      const newline = source.indexOf('\n', index + 2);
      index = newline === -1 ? source.length : newline;
      continue;
    }
    if (source.startsWith('/*', index)) {
      let commentDepth = 1;
      index += 2;
      while (index < source.length && commentDepth > 0) {
        if (source.startsWith('/*', index)) {
          commentDepth += 1;
          index += 2;
        } else if (source.startsWith('*/', index)) {
          commentDepth -= 1;
          index += 2;
        } else {
          index += 1;
        }
      }
      if (commentDepth !== 0) fail(`unterminated block comment in ${context}`);
      index -= 1;
      continue;
    }

    const rawString = /^(?:b)?r(#{0,255})"/.exec(source.slice(index));
    if (rawString) {
      const terminator = `"${rawString[1]}`;
      const end = source.indexOf(terminator, index + rawString[0].length);
      if (end === -1) fail(`unterminated raw string in ${context}`);
      index = end + terminator.length - 1;
      continue;
    }
    const quoteOffset = source.startsWith('b"', index) ? 1 : 0;
    if (source[index + quoteOffset] === '"') {
      index += quoteOffset + 1;
      while (index < source.length) {
        if (source[index] === '\\') index += 2;
        else if (source[index] === '"') break;
        else index += 1;
      }
      if (index >= source.length) fail(`unterminated string in ${context}`);
      continue;
    }
    const character = /^'(?:\\.|[^'\\\r\n])'/.exec(source.slice(index));
    if (character) {
      index += character[0].length - 1;
      continue;
    }

    const token = /^[A-Za-z_][A-Za-z0-9_]*/.exec(source.slice(index));
    if (token) {
      if (token[0] === identifier) indices.push(index);
      index += token[0].length - 1;
    }
  }
  return indices;
}

function rustNamedFunctionSections(source, name, context) {
  const functionIndices = new Set(rustIdentifierIndices(source, 'fn', context));
  const pattern = new RegExp(`\\bfn\\s+${name}\\b`, 'g');
  return [...source.matchAll(pattern)]
    .filter((match) => functionIndices.has(match.index))
    .map((match) => ({
      start: match.index,
      end: rustFunctionEnd(source, match.index, source.length, context),
    }));
}

function handlerRegistrationIndices(path, source, identifier) {
  if (path !== 'apps/desktop/src-tauri/src/main.rs') return [];
  const indices = [];
  const codeIdentifierIndices = new Set(rustIdentifierIndices(source, identifier, path));
  for (const handler of source.matchAll(/\b(?:tauri\s*::\s*)?generate_handler\s*!\s*\[([\s\S]*?)\]/g)) {
    const referencePattern = new RegExp(`\\bcommands\\s*::\\s*${identifier}\\b`, 'g');
    for (const reference of handler[0].matchAll(referencePattern)) {
      const index = handler.index + reference.index + reference[0].lastIndexOf(identifier);
      if (codeIdentifierIndices.has(index)) indices.push(index);
    }
  }
  return indices;
}

export function verifyRemoteOperationsSourceText({
  tauriProductionSources,
  coreProductionSources,
  protocolSource,
  clientSource,
  remoteProductionSources,
  sshProductionSources,
  sshCargoSource,
  readOnlyCommandSource,
  capabilitySource,
}) {
  const commandSections = tauriProductionSources.flatMap(({ path, source }) => {
    const attributePatterns = [/#\s*\[\s*(?:::)?\s*tauri\s*::\s*command\b(?:\s*\([^)]*\))?\s*\]/g];
    const importedCommandNames = new Set();
    for (const match of source.matchAll(
      /\buse\s+(?:::)?\s*tauri\s*::\s*command(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;/g,
    )) {
      importedCommandNames.add(match[1] ?? 'command');
    }
    for (const match of source.matchAll(
      /\buse\s+(?:::)?\s*tauri\s*::\s*\{([^}]*)\}\s*;/g,
    )) {
      for (const item of match[1].split(',')) {
        const commandImport = /^\s*command(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*$/.exec(item);
        if (commandImport) importedCommandNames.add(commandImport[1] ?? 'command');
      }
    }
    for (const attributeName of importedCommandNames) {
      attributePatterns.push(new RegExp(
        `#\\s*\\[\\s*${attributeName}(?:\\s*\\([^)]*\\))?\\s*\\]`,
        'g',
      ));
    }
    for (const match of source.matchAll(
      /\b(?:use|extern\s+crate)\s+tauri\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/g,
    )) {
      attributePatterns.push(new RegExp(
        `#\\s*\\[\\s*${match[1]}\\s*::\\s*command\\b(?:\\s*\\([^)]*\\))?\\s*\\]`,
        'g',
      ));
    }

    const attributes = attributePatterns.flatMap((pattern) =>
      [...source.matchAll(pattern)].map((match) => ({ index: match.index, end: match.index + match[0].length })))
      .sort((left, right) => left.index - right.index);

    return attributes.map((attribute, index) => {
      const end = attributes[index + 1]?.index ?? source.length;
      const declaration = /\b(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/.exec(
        source.slice(attribute.end, end),
      );
      if (!declaration) fail(`Tauri command declaration is missing after attribute in ${path}`);
      const declarationStart = attribute.end + declaration.index;
      const declarationNameStart = declarationStart + declaration[0].lastIndexOf(declaration[1]);
      const functionEnd = rustFunctionEnd(source, declarationStart, end, path);
      return {
        path,
        name: declaration[1],
        source: source.slice(attribute.index, functionEnd),
        start: attribute.index,
        end: functionEnd,
        declarationNameStart,
      };
    });
  });
  const commandNames = commandSections.map(({ name }) => name);
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
  const reviewedCommandPath = 'apps/desktop/src-tauri/src/commands.rs';
  const applicationMutationOperations = [
    { name: 'plan_service_reset_failed', method: 'plan_service_reset_failed' },
    { name: 'discard_service_reset_failed', method: 'discard_service_reset_failed' },
    { name: 'execute_service_reset_failed', method: 'execute_service_reset_failed' },
  ];
  const reviewedDeclarations = applicationMutationOperations.map(({ name, method }) => {
    const command = commandSections.find(({ path, name: commandName }) =>
      path === reviewedCommandPath && commandName === name);
    return command ? { path: command.path, method, index: command.declarationNameStart } : null;
  }).filter(Boolean);
  const reviewedRegistrations = tauriProductionSources.flatMap(({ path, source }) =>
    applicationMutationOperations.flatMap(({ method }) =>
      handlerRegistrationIndices(path, source, method).map((index) => ({ path, method, index }))));
  const applicationMutationReferences = tauriProductionSources.flatMap(({ path, source }) =>
    applicationMutationOperations.flatMap(({ method }) =>
      rustIdentifierIndices(source, method, path)
        .filter((index) => !reviewedDeclarations.some((declaration) =>
          declaration.path === path && declaration.method === method && declaration.index === index))
        .filter((index) => !reviewedRegistrations.some((registration) =>
          registration.path === path && registration.method === method && registration.index === index))
        .map((index) => ({ path, method, index }))));
  const applicationReferencesAreReviewed = reviewedDeclarations.length === 3
    && reviewedRegistrations.length === 3
    && applicationMutationReferences.length === 3
    && applicationMutationOperations.every(({ name, method }) => {
      const command = commandSections.find(({ path, name: commandName }) =>
        path === reviewedCommandPath && commandName === name);
      const registrations = reviewedRegistrations.filter((registration) =>
        registration.method === method);
      const references = applicationMutationReferences.filter((reference) =>
        reference.method === method);
      return command
        && registrations.length === 1
        && references.length === 1
        && references[0].path === reviewedCommandPath
        && references[0].index >= command.start
        && references[0].index < command.end;
    });
  if (!applicationReferencesAreReviewed) {
    fail('Goal 05B application mutation references must remain inside the three reviewed Tauri commands');
  }
  const mutationSurfaceCommands = commandSections.filter(({ source }) =>
    mutationSurfaceFragments.some((fragment) => source.includes(fragment)));
  if (mutationSurfaceCommands.length !== 3
      || mutationSurfaceCommands.some(({ path, name }) =>
        path !== reviewedCommandPath || !allowedMutationCommands.has(name))
      || [...allowedMutationCommands].some((allowed) =>
        mutationSurfaceCommands.filter(({ path, name }) =>
          path === reviewedCommandPath && name === allowed).length !== 1)) {
    fail('only the three reviewed service reset-failed commands may expose mutation authority');
  }

  const reviewedCorePath = 'crates/nexus-core/src/application/services.rs';
  const reviewedCoreOperations = [
    {
      name: 'plan_service_reset_failed',
      foundationOperation: 'plan',
      dtoNames: ['ServiceResetFailedPlan'],
    },
    {
      name: 'discard_service_reset_failed',
      foundationOperation: 'discard',
      dtoNames: [],
    },
    {
      name: 'execute_service_reset_failed',
      foundationOperation: 'execute',
      dtoNames: [
        'ServiceResetFailedResult',
        'ServiceResetFailedOutcome',
        'ServiceResetFailedAuditStatus',
        'ServiceResetFailedPostObservationStatus',
      ],
    },
  ];
  const reviewedCoreDefinitions = reviewedCoreOperations.map((operation) => {
    const definitions = coreProductionSources.flatMap(({ path, source }) => {
      const pattern = new RegExp(`\\bpub\\s+async\\s+fn\\s+${operation.name}\\b`, 'g');
      return [...source.matchAll(pattern)].map((match) => ({
        path,
        source,
        name: operation.name,
        start: match.index,
        declarationNameStart: match.index + match[0].lastIndexOf(operation.name),
        end: rustFunctionEnd(source, match.index, source.length, path),
      }));
    });
    if (definitions.length !== 1 || definitions[0].path !== reviewedCorePath) {
      fail('Goal 05B nexus-core application boundary must remain the three reviewed methods');
    }
    return { ...operation, ...definitions[0] };
  });

  const coreMethodReferences = coreProductionSources.flatMap(({ path, source }) =>
    reviewedCoreOperations.flatMap(({ name }) =>
      rustIdentifierIndices(source, name, path).map((index) => ({ path, name, index }))));
  if (coreMethodReferences.length !== reviewedCoreDefinitions.length
      || coreMethodReferences.some((reference) => !reviewedCoreDefinitions.some((definition) =>
        reference.path === definition.path
        && reference.name === definition.name
        && reference.index === definition.declarationNameStart))) {
    fail('Goal 05B nexus-core application boundary must not have alternate wrappers or references');
  }

  const applicationSource = coreProductionSources.find(({ path }) =>
    path === 'crates/nexus-core/src/application.rs');
  if (!applicationSource) {
    fail('Goal 05B nexus-core remote-operation foundation access is missing Application');
  }
  const reviewedFoundationFragments = [
    'use nexus_remote_operations::RemoteOperationFoundation;',
    'pub(crate) remote_operations: RemoteOperationFoundation,',
    'remote_operations: RemoteOperationFoundation::default(),',
  ];
  const reviewedFoundationTypeIndices = reviewedFoundationFragments.map((fragment) => {
    const start = applicationSource.source.indexOf(fragment);
    if (start === -1 || applicationSource.source.indexOf(fragment, start + 1) !== -1) {
      fail('Goal 05B nexus-core remote-operation foundation access must remain fixed');
    }
    return start + fragment.lastIndexOf('RemoteOperationFoundation');
  });
  const foundationTypeReferences = coreProductionSources.flatMap(({ path, source }) =>
    rustIdentifierIndices(source, 'RemoteOperationFoundation', path)
      .map((index) => ({ path, index })));
  if (foundationTypeReferences.length !== reviewedFoundationTypeIndices.length
      || foundationTypeReferences.some((reference) =>
        reference.path !== applicationSource.path
        || !reviewedFoundationTypeIndices.includes(reference.index))) {
    fail('Goal 05B nexus-core remote-operation foundation access must not be aliased or exported');
  }

  const reviewedFoundationAccesses = [
    { path: 'crates/nexus-core/src/application.rs', name: 'open', count: 1 },
    { path: 'crates/nexus-core/src/application.rs', name: 'shutdown', count: 1 },
    { path: 'crates/nexus-core/src/application/connection.rs', name: 'connect_host', count: 2 },
    { path: 'crates/nexus-core/src/application/rotation.rs', name: 'plan_host_key_rotation', count: 1 },
    { path: 'crates/nexus-core/src/application/rotation.rs', name: 'execute_host_key_rotation', count: 1 },
    { path: reviewedCorePath, name: 'list_host_services', count: 1 },
    { path: reviewedCorePath, name: 'plan_service_reset_failed', count: 2 },
    { path: reviewedCorePath, name: 'discard_service_reset_failed', count: 2 },
    { path: reviewedCorePath, name: 'execute_service_reset_failed', count: 1 },
    { path: 'crates/nexus-core/src/application/lifecycle.rs', name: 'get_session', count: 2 },
    { path: 'crates/nexus-core/src/application/lifecycle.rs', name: 'disconnect_host', count: 1 },
    { path: 'crates/nexus-core/src/application/lifecycle.rs', name: 'prepare_disconnect', count: 1 },
    { path: 'crates/nexus-core/src/application/hosts.rs', name: 'save_host', count: 2 },
    { path: 'crates/nexus-core/src/application/hosts.rs', name: 'delete_host', count: 1 },
    { path: 'crates/nexus-core/src/application/security.rs', name: 'get_host_ssh_trust', count: 1 },
    { path: 'crates/nexus-core/src/application/identity.rs', name: 'trust_host_key', count: 1 },
  ];
  const reviewedFoundationFieldFragment = 'pub(crate) remote_operations: RemoteOperationFoundation,';
  const reviewedFoundationFieldIndex = applicationSource.source.indexOf(
    reviewedFoundationFieldFragment,
  ) + reviewedFoundationFieldFragment.indexOf('remote_operations');
  const allowedFoundationFieldReferences = new Set([
    `${applicationSource.path}:${reviewedFoundationFieldIndex}`,
  ]);
  for (const reviewed of reviewedFoundationAccesses) {
    const file = coreProductionSources.find(({ path }) => path === reviewed.path);
    if (!file) {
      fail('Goal 05B nexus-core remote-operation foundation access is missing a reviewed source');
    }
    const sections = rustNamedFunctionSections(file.source, reviewed.name, file.path);
    if (sections.length !== 1) {
      fail(`Goal 05B nexus-core remote-operation foundation access requires ${reviewed.name}`);
    }
    const references = rustIdentifierIndices(file.source, 'remote_operations', file.path)
      .filter((index) => index >= sections[0].start && index < sections[0].end);
    if (references.length !== reviewed.count) {
      fail(`Goal 05B nexus-core remote-operation foundation access changed in ${reviewed.name}`);
    }
    for (const index of references) {
      allowedFoundationFieldReferences.add(`${file.path}:${index}`);
    }
  }
  const foundationFieldReferences = coreProductionSources.flatMap(({ path, source }) =>
    rustIdentifierIndices(source, 'remote_operations', path)
      .map((index) => `${path}:${index}`));
  if (foundationFieldReferences.length !== allowedFoundationFieldReferences.size
      || foundationFieldReferences.some((reference) =>
        !allowedFoundationFieldReferences.has(reference))) {
    fail('Goal 05B nexus-core remote-operation foundation access must remain in reviewed functions');
  }

  for (const definition of reviewedCoreDefinitions) {
    const operationReferences = rustIdentifierIndices(
      definition.source,
      definition.foundationOperation,
      definition.path,
    ).filter((index) => index >= definition.start && index < definition.end);
    if (operationReferences.length !== 1) {
      fail(`Goal 05B nexus-core application boundary for ${definition.name} must remain fixed`);
    }

    for (const dtoName of definition.dtoNames) {
      const dtoReferences = coreProductionSources.flatMap(({ path, source }) =>
        rustIdentifierIndices(source, dtoName, path).map((index) => ({ path, index })));
      if (dtoReferences.length === 0
          || dtoReferences.some((reference) =>
            reference.path !== definition.path
            || reference.index < definition.start
            || reference.index >= definition.end)) {
        fail(`Goal 05B renderer-facing DTO ${dtoName} must remain in ${definition.name}`);
      }
    }
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
    if (!rendererSurface.includes(required)
        && !tauriProductionSources.some(({ source }) => source.includes(required))) {
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
    tauriProductionSources: await rustSources(root, join(root, 'apps/desktop/src-tauri/src')),
    coreProductionSources: (await rustSources(root, join(root, 'crates/nexus-core/src')))
      .filter(({ path }) => !path.endsWith('/tests.rs') && !path.includes('/tests/')),
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

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

function requireSingleReviewedExecPath(text, context) {
  const execCalls = [...text.matchAll(/\b[A-Za-z_][A-Za-z0-9_]*\s*\.\s*exec\s*\(/g)];
  const targetReferences = text.match(/authority\.target\(\)\.as_str\(\)/g) ?? [];
  if (execCalls.length !== 1
      || targetReferences.length !== 1
      || !text.includes('format!("{COMMAND_PREFIX}{}", authority.target().as_str())')
      || !text.includes('channel.exec(true, command(authority)).await.is_err()')) {
    fail(`${context} must contain exactly the reviewed typed-authority exec path`);
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
  stopImpactQuerySource,
  stopImpactDiscoverySource,
  stopImpactModelSource,
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
    'plan_service_try_restart',
    'discard_service_try_restart',
    'execute_service_try_restart',
    'plan_service_reload',
    'discard_service_reload',
    'execute_service_reload',
    'plan_service_start',
    'discard_service_start',
    'execute_service_start',
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
    'ServiceTryRestartPlan',
    'ServiceTryRestartResult',
    'ServiceTryRestartOutcome',
    'ServiceTryRestartAuditStatus',
    'ServiceTryRestartPostObservationStatus',
    'ServiceReloadPlan',
    'ServiceReloadResult',
    'ServiceReloadOutcome',
    'ServiceReloadAuditStatus',
    'ServiceReloadPostObservationStatus',
    'ServiceStartPlan',
    'ServiceStartResult',
    'ServiceStartOutcome',
    'ServiceStartAuditStatus',
    'ServiceStartPostObservationStatus',
    '.plan_service_reset_failed(',
    '.discard_service_reset_failed(',
    '.execute_service_reset_failed(',
    '.plan_service_try_restart(',
    '.discard_service_try_restart(',
    '.execute_service_try_restart(',
    '.plan_service_reload(',
    '.discard_service_reload(',
    '.execute_service_reload(',
    '.plan_service_start(',
    '.discard_service_start(',
    '.execute_service_start(',
  ];
  const reviewedCommandPath = 'apps/desktop/src-tauri/src/commands.rs';
  const applicationMutationOperations = [
    { name: 'plan_service_reset_failed', method: 'plan_service_reset_failed' },
    { name: 'discard_service_reset_failed', method: 'discard_service_reset_failed' },
    { name: 'execute_service_reset_failed', method: 'execute_service_reset_failed' },
    { name: 'plan_service_try_restart', method: 'plan_service_try_restart' },
    { name: 'discard_service_try_restart', method: 'discard_service_try_restart' },
    { name: 'execute_service_try_restart', method: 'execute_service_try_restart' },
    { name: 'plan_service_reload', method: 'plan_service_reload' },
    { name: 'discard_service_reload', method: 'discard_service_reload' },
    { name: 'execute_service_reload', method: 'execute_service_reload' },
    { name: 'plan_service_start', method: 'plan_service_start' },
    { name: 'discard_service_start', method: 'discard_service_start' },
    { name: 'execute_service_start', method: 'execute_service_start' },
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
  const applicationReferencesAreReviewed = reviewedDeclarations.length === 12
    && reviewedRegistrations.length === 12
    && applicationMutationReferences.length === 12
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
    fail('Goal 05B/05D/05E/05F application mutation references must remain inside the twelve reviewed Tauri commands');
  }
  const mutationSurfaceCommands = commandSections.filter(({ source }) =>
    mutationSurfaceFragments.some((fragment) => source.includes(fragment)));
  if (mutationSurfaceCommands.length !== 12
      || mutationSurfaceCommands.some(({ path, name }) =>
        path !== reviewedCommandPath || !allowedMutationCommands.has(name))
      || [...allowedMutationCommands].some((allowed) =>
        mutationSurfaceCommands.filter(({ path, name }) =>
          path === reviewedCommandPath && name === allowed).length !== 1)) {
    fail('only the twelve reviewed service mutation commands may expose mutation authority');
  }
  requireAbsent(
    mutationSurfaceCommands.map(({ source }) => source).join('\n'),
    [
      'unit: String', 'can_start: bool', 'can_reload: bool', 'risk: OperationRisk',
      'timeout:', 'command:', 'verb:', 'argv:', 'environment:',
    ],
    'reviewed service mutation Tauri commands',
  );

  const reviewedCorePath = 'crates/nexus-core/src/application/services.rs';
  const reviewedCoreOperations = [
    {
      name: 'plan_service_reset_failed',
      foundationOperation: 'plan',
      nativeType: 'SystemdResetFailed',
      dtoNames: ['ServiceResetFailedPlan'],
    },
    {
      name: 'discard_service_reset_failed',
      foundationOperation: 'discard',
      nativeType: 'SystemdResetFailed',
      dtoNames: [],
    },
    {
      name: 'execute_service_reset_failed',
      foundationOperation: 'execute',
      nativeType: 'SystemdResetFailed',
      dtoNames: [
        'ServiceResetFailedResult',
        'ServiceResetFailedOutcome',
        'ServiceResetFailedAuditStatus',
        'ServiceResetFailedPostObservationStatus',
      ],
    },
    {
      name: 'plan_service_try_restart',
      foundationOperation: 'plan',
      nativeType: 'SystemdTryRestart',
      dtoNames: ['ServiceTryRestartPlan'],
    },
    {
      name: 'discard_service_try_restart',
      foundationOperation: 'discard',
      nativeType: 'SystemdTryRestart',
      dtoNames: [],
    },
    {
      name: 'execute_service_try_restart',
      foundationOperation: 'execute',
      nativeType: 'SystemdTryRestart',
      dtoNames: [
        'ServiceTryRestartResult',
        'ServiceTryRestartOutcome',
        'ServiceTryRestartAuditStatus',
        'ServiceTryRestartPostObservationStatus',
      ],
    },
    {
      name: 'plan_service_reload',
      foundationOperation: 'plan',
      nativeType: 'SystemdReload',
      dtoNames: ['ServiceReloadPlan'],
    },
    {
      name: 'discard_service_reload',
      foundationOperation: 'discard',
      nativeType: 'SystemdReload',
      dtoNames: [],
    },
    {
      name: 'execute_service_reload',
      foundationOperation: 'execute',
      nativeType: 'SystemdReload',
      dtoNames: [
        'ServiceReloadResult',
        'ServiceReloadOutcome',
        'ServiceReloadAuditStatus',
        'ServiceReloadPostObservationStatus',
      ],
    },
    {
      name: 'plan_service_start',
      foundationOperation: 'plan',
      nativeType: 'SystemdStart',
      dtoNames: ['ServiceStartPlan'],
    },
    {
      name: 'discard_service_start',
      foundationOperation: 'discard',
      nativeType: 'SystemdStart',
      dtoNames: [],
    },
    {
      name: 'execute_service_start',
      foundationOperation: 'execute',
      nativeType: 'SystemdStart',
      dtoNames: [
        'ServiceStartResult',
        'ServiceStartOutcome',
        'ServiceStartAuditStatus',
        'ServiceStartPostObservationStatus',
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
      fail('Goal 05B/05D/05E/05F nexus-core application boundary must remain the twelve reviewed methods');
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
    { path: reviewedCorePath, name: 'assess_service_stop_impact', count: 1 },
    { path: reviewedCorePath, name: 'plan_service_reset_failed', count: 2 },
    { path: reviewedCorePath, name: 'discard_service_reset_failed', count: 2 },
    { path: reviewedCorePath, name: 'execute_service_reset_failed', count: 1 },
    { path: reviewedCorePath, name: 'plan_service_try_restart', count: 2 },
    { path: reviewedCorePath, name: 'discard_service_try_restart', count: 2 },
    { path: reviewedCorePath, name: 'execute_service_try_restart', count: 1 },
    { path: reviewedCorePath, name: 'plan_service_reload', count: 2 },
    { path: reviewedCorePath, name: 'discard_service_reload', count: 2 },
    { path: reviewedCorePath, name: 'execute_service_reload', count: 1 },
    { path: reviewedCorePath, name: 'plan_service_start', count: 2 },
    { path: reviewedCorePath, name: 'discard_service_start', count: 2 },
    { path: reviewedCorePath, name: 'execute_service_start', count: 1 },
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
    const nativeTypeReferences = rustIdentifierIndices(
      definition.source,
      definition.nativeType,
      definition.path,
    ).filter((index) => index >= definition.start && index < definition.end);
    if (nativeTypeReferences.length !== 1) {
      fail(`Goal 05B/05D native operation binding for ${definition.name} must remain fixed`);
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
      'ServiceOperationPlan',
      'ServiceActionPlan',
      'serviceOperationVerb',
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
    'allow-plan-service-try-restart',
    'allow-discard-service-try-restart',
    'allow-execute-service-try-restart',
    'allow-plan-service-reload',
    'allow-discard-service-reload',
    'allow-execute-service-reload',
    'allow-plan-service-start',
    'allow-discard-service-start',
    'allow-execute-service-start',
  ]) {
    if (!capabilitySource.includes(`"${permission}"`)) {
      fail(`reviewed service mutation capability is missing ${permission}`);
    }
  }

  const nativeImplementations = remoteProduction.match(/impl NativeOperation for/g) ?? [];
  const tryRestartOperation = remoteProductionSources.find(({ path }) =>
    path === 'crates/nexus-remote-operations/src/systemd_try_restart.rs');
  if (!tryRestartOperation) fail('reviewed SystemdTryRestart operation module is missing');
  const tryRestartOperationProduction = tryRestartOperation.source.split('#[cfg(test)]')[0];
  const reloadOperation = remoteProductionSources.find(({ path }) =>
    path === 'crates/nexus-remote-operations/src/systemd_reload.rs');
  if (!reloadOperation) fail('reviewed SystemdReload operation module is missing');
  const reloadOperationProduction = reloadOperation.source.split('#[cfg(test)]')[0];
  const startOperation = remoteProductionSources.find(({ path }) =>
    path === 'crates/nexus-remote-operations/src/systemd_start.rs');
  if (!startOperation) fail('reviewed SystemdStart operation module is missing');
  const startOperationProduction = startOperation.source.split('#[cfg(test)]')[0];
  if (nativeImplementations.length !== 4
      || !systemdOperationProduction.includes('impl NativeOperation for SystemdResetFailed')
      || !tryRestartOperationProduction.includes('impl NativeOperation for SystemdTryRestart')
      || !reloadOperationProduction.includes('impl NativeOperation for SystemdReload')
      || !startOperationProduction.includes('impl NativeOperation for SystemdStart')) {
    fail('private remote-operation production source must contain exactly SystemdResetFailed, SystemdTryRestart, SystemdReload and SystemdStart');
  }
  for (const required of [
    'SystemdResetFailedPreconditions',
    'const RISK: OperationRisk = OperationRisk::Moderate;',
    'const MAX_UNIT_BYTES: usize = 255;',
  ]) {
    if (!systemdOperationProduction.includes(required)) fail(`systemd operation policy is missing ${required}`);
  }
  for (const required of [
    'SystemdTryRestartPreconditions',
    'const RISK: OperationRisk = OperationRisk::High;',
    'load == "loaded" && active == "active" && sub == "running"',
  ]) {
    if (!tryRestartOperationProduction.includes(required)) fail(`try-restart operation policy is missing ${required}`);
  }
  for (const required of [
    'SystemdReloadPreconditions',
    'const RISK: OperationRisk = OperationRisk::High;',
    'load == "loaded" && active == "active" && sub == "running" && can_reload',
  ]) {
    if (!reloadOperationProduction.includes(required)) fail(`reload operation policy is missing ${required}`);
  }
  for (const required of [
    'SystemdStartPreconditions',
    'const RISK: OperationRisk = OperationRisk::High;',
    'load == "loaded" && active == "inactive" && sub == "dead" && can_start',
  ]) {
    if (!startOperationProduction.includes(required)) fail(`start operation policy is missing ${required}`);
  }

  if (!sshCargoSource.includes('nexus-remote-operations = { path = "../nexus-remote-operations" }')) {
    fail('SSH production source is missing the reviewed remote-operation dependency');
  }
  const systemdTransport = sshProductionSources.find(({ path }) =>
    path === 'crates/nexus-ssh/src/systemd_reset_failed.rs');
  if (!systemdTransport) fail('reviewed SystemdResetFailed SSH adapter is missing');
  const tryRestartTransport = sshProductionSources.find(({ path }) =>
    path === 'crates/nexus-ssh/src/systemd_try_restart.rs');
  if (!tryRestartTransport) fail('reviewed SystemdTryRestart SSH adapter is missing');
  const reloadTransport = sshProductionSources.find(({ path }) =>
    path === 'crates/nexus-ssh/src/systemd_reload.rs');
  if (!reloadTransport) fail('reviewed SystemdReload SSH adapter is missing');
  const startTransport = sshProductionSources.find(({ path }) =>
    path === 'crates/nexus-ssh/src/systemd_start.rs');
  if (!startTransport) fail('reviewed SystemdStart SSH adapter is missing');
  const systemdTransportSource = systemdTransport.source;
  const systemdTransportProduction = systemdTransportSource.split('#[cfg(test)]')[0];
  const otherSshSources = sshProductionSources
    .filter(({ path }) => path !== systemdTransport.path
      && path !== tryRestartTransport.path
      && path !== reloadTransport.path
      && path !== startTransport.path)
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
    'Some(message) => match handle_message(',
    'ChannelMsg::ExitStatus { exit_status } =>',
    'ChannelMsg::ExitSignal { .. } =>',
    'ChannelMsg::Success if !*accepted =>',
    'ChannelMsg::Failure if !*accepted && !*execution_evidence =>',
    'ChannelMsg::Failure => MessageHandling::Complete(',
    'ChannelMsg::Data { data }',
    'ChannelMsg::ExtendedData { data, .. } =>',
    '*execution_evidence = true;',
    '*output_bytes = output_bytes.saturating_add(data.len());',
    'if *output_bytes > OUTPUT_LIMIT',
    'CompletionUnknownReason::OutputLimit',
    'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
    'ChannelMsg::Close => MessageHandling::Complete(',
    '_ if !*accepted => MessageHandling::Complete(',
    'None => {',
    'CompletionUnknownReason::Cancelled',
    'CompletionUnknownReason::Timeout',
  ]) {
    if (!systemdTransportProduction.includes(required)) fail(`systemd SSH transport policy is missing ${required}`);
  }
  requireAbsent(
    systemdTransportProduction,
    [
      'sudo',
      'pkexec',
      'Vec<SystemdServiceUnitName>',
      'command: String',
      'verb: String',
      'ChannelMsg::ExitStatus { exit_status } if *accepted =>',
      'ChannelMsg::ExitSignal { .. } if *accepted =>',
      'ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close',
      'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close',
    ],
    'reviewed systemd SSH transport',
  );
  if (!systemdTransportProduction.includes('authority.target().as_str()')) {
    fail('systemd SSH transport must append exactly one typed authority target');
  }
  requireSingleReviewedExecPath(systemdTransportProduction, 'reviewed systemd SSH transport');
  const tryRestartTransportProduction = tryRestartTransport.source.split('#[cfg(test)]')[0];
  for (const required of [
    'impl MutationTransport<SystemdTryRestart> for SshSession',
    'const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);',
    'const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);',
    'const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);',
    'const OUTPUT_LIMIT: usize = 8 * 1024;',
    'LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password try-restart -- ',
    'Some(message) => match handle_message(',
    'ChannelMsg::Success if !*accepted =>',
    'ChannelMsg::Failure if !*accepted && !*execution_evidence =>',
    'ChannelMsg::Failure => MessageHandling::Complete(',
    'ChannelMsg::Data { data }',
    'ChannelMsg::ExtendedData { data, .. } =>',
    '*execution_evidence = true;',
    '*output_bytes = output_bytes.saturating_add(data.len());',
    'if *output_bytes > OUTPUT_LIMIT',
    'CompletionUnknownReason::OutputLimit',
    'ChannelMsg::ExitStatus { exit_status } =>',
    'ChannelMsg::ExitSignal { .. } =>',
    'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
    'ChannelMsg::Close => MessageHandling::Complete(',
    '_ if !*accepted => MessageHandling::Complete(',
    'None => {',
    'CompletionUnknownReason::Cancelled',
    'CompletionUnknownReason::Timeout',
    'authority.target().as_str()',
  ]) {
    if (!tryRestartTransportProduction.includes(required)) fail(`try-restart SSH transport policy is missing ${required}`);
  }
  requireAbsent(
    tryRestartTransportProduction,
    [
      'sudo', 'pkexec', 'Vec<SystemdServiceUnitName>', 'command: String', 'verb: String',
      'systemctl restart', 'systemctl start', 'systemctl stop', 'systemctl reload',
      'ChannelMsg::ExitStatus { exit_status } if *accepted =>',
      'ChannelMsg::ExitSignal { .. } if *accepted =>',
      'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close',
    ],
    'reviewed try-restart SSH transport',
  );
  requireSingleReviewedExecPath(
    tryRestartTransportProduction,
    'reviewed try-restart SSH transport',
  );
  const reloadTransportProduction = reloadTransport.source.split('#[cfg(test)]')[0];
  for (const required of [
    'impl MutationTransport<SystemdReload> for SshSession',
    'const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);',
    'const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);',
    'const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);',
    'const OUTPUT_LIMIT: usize = 8 * 1024;',
    'LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password reload -- ',
    'Some(message) => match handle_message(',
    'ChannelMsg::Success if !*accepted =>',
    'ChannelMsg::Failure if !*accepted && !*execution_evidence =>',
    'ChannelMsg::Failure => MessageHandling::Complete(',
    'ChannelMsg::Data { data }',
    'ChannelMsg::ExtendedData { data, .. } =>',
    '*execution_evidence = true;',
    '*output_bytes = output_bytes.saturating_add(data.len());',
    'if *output_bytes > OUTPUT_LIMIT',
    'CompletionUnknownReason::OutputLimit',
    'ChannelMsg::ExitStatus { exit_status } =>',
    'ChannelMsg::ExitSignal { .. } =>',
    'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
    'ChannelMsg::Close => MessageHandling::Complete(',
    '_ if !*accepted => MessageHandling::Complete(',
    'None => {',
    'CompletionUnknownReason::Cancelled',
    'CompletionUnknownReason::Timeout',
    'authority.target().as_str()',
  ]) {
    if (!reloadTransportProduction.includes(required)) {
      fail(`reload SSH transport policy is missing ${required}`);
    }
  }
  requireAbsent(
    reloadTransportProduction,
    [
      'sudo', 'pkexec', 'Vec<SystemdServiceUnitName>', 'command: String', 'command: &str',
      'verb: String', 'verb: &str', 'systemctl restart', 'systemctl try-restart',
      'reload-or-restart', 'try-reload-or-restart', 'systemctl start', 'systemctl stop',
      'daemon-reload', '--no-block',
      'ChannelMsg::ExitStatus { exit_status } if *accepted =>',
      'ChannelMsg::ExitSignal { .. } if *accepted =>',
      'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close',
    ],
    'reviewed reload SSH transport',
  );
  requireSingleReviewedExecPath(reloadTransportProduction, 'reviewed reload SSH transport');
  const startTransportProduction = startTransport.source.split('#[cfg(test)]')[0];
  for (const required of [
    'impl MutationTransport<SystemdStart> for SshSession',
    'const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);',
    'const REQUEST_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);',
    'const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);',
    'const OUTPUT_LIMIT: usize = 8 * 1024;',
    'LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password start -- ',
    'Some(message) => match handle_message(',
    'ChannelMsg::Success if !*accepted =>',
    'ChannelMsg::Failure if !*accepted && !*execution_evidence =>',
    'ChannelMsg::Failure => MessageHandling::Complete(',
    'ChannelMsg::Data { data }',
    'ChannelMsg::ExtendedData { data, .. } =>',
    '*execution_evidence = true;',
    '*output_bytes = output_bytes.saturating_add(data.len());',
    'if *output_bytes > OUTPUT_LIMIT',
    'CompletionUnknownReason::OutputLimit',
    'ChannelMsg::ExitStatus { exit_status } =>',
    'ChannelMsg::ExitSignal { .. } =>',
    'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } => MessageHandling::Continue,',
    'ChannelMsg::Close => MessageHandling::Complete(',
    '_ if !*accepted => MessageHandling::Complete(',
    'None => {',
    'CompletionUnknownReason::Cancelled',
    'CompletionUnknownReason::Timeout',
    'authority.target().as_str()',
  ]) {
    if (!startTransportProduction.includes(required)) {
      fail(`start SSH transport policy is missing ${required}`);
    }
  }
  requireAbsent(
    startTransportProduction,
    [
      'sudo', 'pkexec', 'Vec<SystemdServiceUnitName>', 'command: String', 'command: &str',
      'verb: String', 'verb: &str', 'systemctl restart', 'systemctl try-restart',
      'reload-or-restart', 'try-reload-or-restart', 'systemctl reload', 'systemctl stop',
      'daemon-reload', '--no-block',
      'ChannelMsg::ExitStatus { exit_status } if *accepted =>',
      'ChannelMsg::ExitSignal { .. } if *accepted =>',
      'ChannelMsg::Eof | ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Close',
    ],
    'reviewed start SSH transport',
  );
  requireSingleReviewedExecPath(startTransportProduction, 'reviewed start SSH transport');
  for (const required of [
    'plan_service_reset_failed',
    'discard_service_reset_failed',
    'execute_service_reset_failed',
    'ServiceResetFailedPlan',
    'ServiceResetFailedResult',
    'plan_service_try_restart',
    'discard_service_try_restart',
    'execute_service_try_restart',
    'ServiceTryRestartPlan',
    'ServiceTryRestartResult',
    'plan_service_reload',
    'discard_service_reload',
    'execute_service_reload',
    'ServiceReloadPlan',
    'ServiceReloadResult',
    'plan_service_start',
    'discard_service_start',
    'execute_service_start',
    'ServiceStartPlan',
    'ServiceStartResult',
  ]) {
    if (!rendererSurface.includes(required)
        && !tauriProductionSources.some(({ source }) => source.includes(required))) {
      fail(`reviewed service mutation surface is missing ${required}`);
    }
  }

  const readOnlyDeclaration = /#\[derive\(([^)]*)\)\]\s*pub enum ReadOnlyCommand/.exec(
    readOnlyCommandSource,
  );
  const serviceInventoryCommand = 'LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --all --type=service --property=Id --property=LoadState --property=ActiveState --property=SubState --property=CanStart --property=CanReload --property=Description show';
  const readOnlyProduction = readOnlyCommandSource.split('#[cfg(test)]')[0];
  if (!readOnlyProduction.includes(serviceInventoryCommand)
      || readOnlyProduction.indexOf(serviceInventoryCommand)
        !== readOnlyProduction.lastIndexOf(serviceInventoryCommand)) {
    fail('service inventory command must remain fixed with strict CanStart and CanReload observation');
  }
  if (!readOnlyDeclaration) fail('ReadOnlyCommand declaration is missing');
  if (/\b(?:Serialize|Deserialize|TS)\b/.test(readOnlyDeclaration[1])) {
    fail('ReadOnlyCommand must remain native and non-serializable');
  }

  const stopImpactCommands = commandSections.filter(({ source }) =>
    source.includes('SystemdStopImpact'));
  if (stopImpactCommands.length !== 1
      || stopImpactCommands[0].path !== reviewedCommandPath
      || stopImpactCommands[0].name !== 'assess_service_stop_impact') {
    fail('Goal 05G must expose exactly one reviewed read-only assessment Tauri command');
  }
  const stopImpactCommand = stopImpactCommands[0].source;
  for (const required of [
    'host_id: HostId',
    'host_session_id: HostSessionId',
    'inspection_id: SystemdStopImpactInspectionId',
    'Result<SystemdStopImpactAssessment, AppError>',
    'app.assess_service_stop_impact(host_id, host_session_id, inspection_id)',
  ]) {
    if (!stopImpactCommand.includes(required)) {
      fail(`Goal 05G assessment Tauri command is missing ${required}`);
    }
  }
  for (const forbidden of [
    'unit:', 'unit_name:', 'service:', 'target:', 'String', 'ServiceObservationId',
    'RemoteOperationPlanId', '.plan_service_', '.execute_service_', '.discard_service_',
  ]) {
    if (stopImpactCommand.includes(forbidden)) {
      fail(`Goal 05G assessment Tauri command contains forbidden authority input ${forbidden}`);
    }
  }
  const stopImpactRegistrations = tauriProductionSources.flatMap(({ path, source }) =>
    handlerRegistrationIndices(path, source, 'assess_service_stop_impact'));
  if (stopImpactRegistrations.length !== 1
      || !capabilitySource.includes('allow-assess-service-stop-impact')) {
    fail('Goal 05G assessment command registration and capability must remain singular');
  }

  const stopImpactQueryProduction = stopImpactQuerySource.split('#[cfg(test)]')[0];
  const expectedStopImpactProperties = [
    'Id', 'Names', 'Following', 'LoadState', 'ActiveState', 'SubState', 'CanStop',
    'RefuseManualStop', 'Job', 'NeedDaemonReload', 'StopWhenUnneeded', 'Requires',
    'RequiredBy', 'Requisite', 'RequisiteOf', 'Wants', 'WantedBy', 'BindsTo',
    'BoundBy', 'PartOf', 'ConsistsOf', 'PropagatesStopTo', 'StopPropagatedFrom',
    'Upholds', 'UpheldBy', 'Conflicts', 'ConflictedBy', 'Before', 'After', 'Triggers',
    'TriggeredBy', 'OnSuccess', 'OnFailure', 'OnSuccessJobMode', 'OnFailureJobMode',
    'SuccessAction', 'FailureAction',
  ];
  const propertyDeclaration = /SYSTEMD_STOP_IMPACT_PROPERTIES:\s*\[&str;\s*37\]\s*=\s*\[([\s\S]*?)\];/.exec(
    stopImpactQueryProduction,
  );
  const actualStopImpactProperties = propertyDeclaration
    ? [...propertyDeclaration[1].matchAll(/"([A-Za-z]+)"/g)].map((match) => match[1])
    : [];
  if (JSON.stringify(actualStopImpactProperties) !== JSON.stringify(expectedStopImpactProperties)) {
    fail('Goal 05G fixed 37-property systemd query contract changed');
  }
  const fixedStopImpactPrefix = 'LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password --all --property=';
  for (const required of [
    fixedStopImpactPrefix,
    'const MAX_UNITS_PER_QUERY: usize = 16;',
    'const MAX_UNIT_BYTES: usize = 255;',
    'const MAX_OUTPUT_BYTES: usize = 64 * 1024;',
    'const QUERY_TIMEOUT: Duration = Duration::from_secs(8);',
    'command.push_str(" show --");',
    'session.execute_stop_impact(query, child)',
  ]) {
    if (!stopImpactQueryProduction.includes(required)) {
      fail(`Goal 05G fixed native read-only query is missing ${required}`);
    }
  }
  for (const forbidden of ['systemctl stop', 'sudo', 'pkexec', 'retry', 'pub fn command_for']) {
    if (stopImpactQueryProduction.toLowerCase().includes(forbidden.toLowerCase())) {
      fail(`Goal 05G native read-only query contains forbidden fragment ${forbidden}`);
    }
  }

  for (const required of [
    'const MAX_NODES: usize = 64;',
    'const MAX_EDGES: usize = 512;',
    'const MAX_DEPTH: u8 = 4;',
    'const MAX_ALIASES: usize = 16;',
    'const MAX_PROPERTY_BYTES: usize = 8 * 1024;',
    'const MAX_BLOCK_BYTES: usize = 32 * 1024;',
    'const MAX_AGGREGATE_OUTPUT_BYTES: usize = 256 * 1024;',
    'const MAX_WORKING_SET_BYTES: usize = 2 * 1024 * 1024;',
    'const MAX_QUERIES: u8 = 6;',
    'const FINAL_QUERY_RESERVE: Duration = Duration::from_secs(8);',
    'const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);',
    'while builder.accounting.actual_ssh_queries < MAX_QUERIES - 1',
    'builder.accounting.actual_ssh_queries += 1;',
    'engine.execute(session, &root_query, cancellation.clone())',
    'self.promote_cached_descendants(canonical);',
    'candidate_affected(',
    'fn match_batch_by_identity(',
    'identities.len() != 1',
    'fn has_direct_frontier(&self) -> bool',
    'fn promote_eligible_candidates(&mut self)',
    'StopImpactConditionalClassification::CoverageUnknown',
    'std::num::NonZeroU32::new',
  ]) {
    if (!stopImpactDiscoverySource.includes(required)) {
      fail(`Goal 05G bounded graph implementation is missing ${required}`);
    }
  }
  const stopImpactDiscoveryProduction = stopImpactDiscoverySource.split('#[cfg(test)]')[0];
  if ((stopImpactDiscoveryProduction.match(/engine\s*\.execute\(session, &root_query/g) ?? []).length !== 2) {
    fail('Goal 05G root must be queried exactly twice: initial observation and final revalidation');
  }
  const sshSessionSource = sshProductionSources.find(({ path }) =>
    path === 'crates/nexus-ssh/src/session.rs')?.source ?? '';
  for (const required of [
    'enum ReviewedReadOnlyCommand',
    'Fixed(ReadOnlyCommand)',
    'StopImpact(&',
    'SystemdStopImpactQuery)',
    'ReviewedReadOnlyCommand::StopImpact(query)',
    'fn rejects_stderr(&self) -> bool',
    'collect_stop_impact_output(&mut reader)',
    'if reject_stderr',
  ]) {
    if (!sshSessionSource.includes(required)) {
      fail(`Goal 05G SSH transport must remain closed over typed read-only commands: ${required}`);
    }
  }
  if (/execute_(?:fixed|reviewed)_read_only\s*\([^)]*(?:&str|String)/s.test(sshSessionSource)) {
    fail('Goal 05G SSH transport must not expose a generic command-string helper');
  }

  const stopImpactMutationLeak = [
    ...remoteProductionSources,
    ...commandSections.filter(({ name }) => allowedMutationCommands.has(name)),
  ].some(({ source }) => source.includes('SystemdStopImpact'));
  if (stopImpactMutationLeak) {
    fail('Goal 05G inspection identity or result must not enter mutation authority');
  }
  if (!stopImpactModelSource.includes('pub struct SystemdStopImpactInspectionId')
      || !stopImpactModelSource.includes('pub canonical_unit: String')
      || /RemoteOperationPlanId|NativeOperation|SystemdStop\b/.test(stopImpactModelSource)) {
    fail('Goal 05G diagnostic DTO boundary changed or gained mutation authority');
  }
  const productionRust = [
    ...tauriProductionSources,
    ...coreProductionSources,
    ...remoteProductionSources,
    ...sshProductionSources,
  ].map(({ source }) => source).join('\n');
  if (/\bSystemdStop\b/.test(productionRust)
      || /(?:plan|execute|discard)_service_stop\b/.test(productionRust)
      || /systemctl\s+stop\b/.test(productionRust)) {
    fail('Goal 05G must not introduce SystemdStop or a fifth mutation surface');
  }
  const rendererAssessment = /assessStopImpact:\s*\(([\s\S]*?)\)\s*=>\s*request<SystemdStopImpactAssessment>/.exec(
    clientSource,
  );
  if (!rendererAssessment
      || !rendererAssessment[1].includes('inspectionId: SystemdStopImpactInspectionId')
      || /unit|service|target|canonical/i.test(rendererAssessment[1])) {
    fail('Goal 05G renderer request must carry only opaque inspection authority');
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
    stopImpactQuerySource: await readFile(join(root, 'crates/nexus-operations/src/stop_impact.rs'), 'utf8'),
    stopImpactDiscoverySource: await readFile(join(root, 'crates/nexus-discovery/src/stop_impact.rs'), 'utf8'),
    stopImpactModelSource: await readFile(join(root, 'crates/nexus-model/src/stop_impact.rs'), 'utf8'),
    capabilitySource: await readFile(join(root, 'apps/desktop/src-tauri/capabilities/main.json'), 'utf8'),
  };
}

export async function verifyRemoteOperationsSource(root) {
  return verifyRemoteOperationsSourceText(await remoteOperationsSourceFixture(root));
}

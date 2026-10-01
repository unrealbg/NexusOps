import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

function fail(message) {
  throw new Error(message);
}

function matching(text, start, open, close) {
  let depth = 0;
  let quote = null;
  let escaped = false;
  for (let index = start; index < text.length; index += 1) {
    const character = text[index];
    if (quote) {
      if (escaped) escaped = false;
      else if (character === '\\') escaped = true;
      else if (character === quote) quote = null;
      continue;
    }
    if (character === '"') {
      quote = character;
      continue;
    }
    if (character === open) depth += 1;
    if (character === close && --depth === 0) return index;
  }
  fail(`unbalanced ${open}${close} in command source`);
}

export function commandDefinitions(source) {
  const commands = [];
  const marker = /#\[tauri::command\]/g;
  for (const match of source.matchAll(marker)) {
    const tail = source.slice(match.index + match[0].length);
    const declaration = /pub\s+(?:async\s+)?fn\s+([a-z0-9_]+)/.exec(tail);
    if (!declaration) fail('Tauri command attribute is not followed by a public function');
    const declarationStart = match.index + match[0].length + declaration.index;
    const parametersStart = source.indexOf('(', declarationStart);
    const parametersEnd = matching(source, parametersStart, '(', ')');
    const bodyStart = source.indexOf('{', parametersEnd);
    if (bodyStart === -1) fail(`${declaration[1]} command body is missing`);
    const bodyEnd = matching(source, bodyStart, '{', '}');
    commands.push({
      name: declaration[1],
      parameters: source.slice(parametersStart + 1, parametersEnd),
      body: source.slice(bodyStart + 1, bodyEnd),
    });
  }
  return commands;
}

export function verifyLifecycleSourceText({
  commandsSource,
  mainSource,
  buildSource,
  capability,
  lifecycleSource,
  updaterSources,
}) {
  const commands = commandDefinitions(commandsSource);
  if (commands.length === 0) fail('no Tauri commands were found');
  const names = commands.map(({ name }) => name);
  if (new Set(names).size !== names.length) fail('Tauri command names must be unique');

  for (const { name, parameters, body } of commands) {
    const compactParameters = parameters.replace(/\s+/g, '');
    if (!compactParameters.includes("lifecycle:State<'_,LifecycleCoordinator>"))
      fail(`${name} must receive the native lifecycle coordinator`);
    const compactBody = body.trimStart();
    if (!compactBody.startsWith('let _permit = lifecycle.admit()?;'))
      fail(`${name} must acquire its lifecycle permit as its first statement`);
    if ((body.match(/lifecycle\.admit\(\)\?/g) ?? []).length !== 1)
      fail(`${name} must acquire exactly one lifecycle permit`);
    if (/drop\s*\(\s*_permit\s*\)/.test(body))
      fail(`${name} must retain its lifecycle permit for the complete invocation`);
  }

  for (const name of names) {
    if ((mainSource.match(new RegExp(`commands::${name}\\b`, 'g')) ?? []).length !== 1)
      fail(`${name} must be registered exactly once in the Tauri invoke handler`);
    if ((buildSource.match(new RegExp(`"${name}"`, 'g')) ?? []).length !== 1)
      fail(`${name} must be registered exactly once in the Tauri app manifest`);
    const permission = `allow-${name.replaceAll('_', '-')}`;
    if (capability.permissions.filter((value) => value === permission).length !== 1)
      fail(`${name} must have exactly one matching custom capability permission`);
  }

  const lifecycleWords = /(lifecycle|seal|drain|shutdown|quiesc|exit)/i;
  if (names.some((name) => lifecycleWords.test(name)))
    fail('renderer lifecycle-control commands are forbidden');
  if (capability.permissions.some((permission) => lifecycleWords.test(permission)))
    fail('renderer lifecycle-control permissions are forbidden');

  for (const fragment of [
    'Running',
    'Sealed',
    'Quiesced',
    'checked_add(1)',
    'checked_sub(1)',
    'Duration::from_secs(60)',
    'tokio::time::timeout_at(deadline, notified)',
    'NexusOps is closing. Restart the application to continue.',
  ]) {
    if (!lifecycleSource.includes(fragment)) fail(`lifecycle policy is missing ${fragment}`);
  }
  const cleanupOrder = [
    '.seal_and_drain()',
    '.shutdown_updates()',
    '.shutdown_application()',
    '.revoke_local_grants()',
    '.finalize_logging()',
  ];
  let previous = -1;
  for (const fragment of cleanupOrder) {
    const index = lifecycleSource.indexOf(fragment, previous + 1);
    if (index === -1) fail(`exit cleanup order is missing ${fragment}`);
    previous = index;
  }
  if (!mainSource.includes('api.prevent_exit();') || !mainSource.includes('tauri::async_runtime::spawn'))
    fail('normal exit must be prevented before asynchronous cleanup starts');
  if (mainSource.includes('tauri::async_runtime::block_on'))
    fail('the Tauri event-loop callback must not block on cleanup');

  const updaterText = updaterSources.join('\n');
  for (const forbidden of ['download_and_install', '.install(']) {
    if (updaterText.includes(forbidden)) fail(`runtime updater source must not contain ${forbidden}`);
  }
  return { commandCount: commands.length };
}

export async function verifyLifecycleSource(root) {
  const desktop = join(root, 'apps/desktop/src-tauri');
  const [commandsSource, mainSource, buildSource, capabilityText, lifecycleSource, updates, download] =
    await Promise.all([
      readFile(join(desktop, 'src/commands.rs'), 'utf8'),
      readFile(join(desktop, 'src/main.rs'), 'utf8'),
      readFile(join(desktop, 'build.rs'), 'utf8'),
      readFile(join(desktop, 'capabilities/main.json'), 'utf8'),
      readFile(join(desktop, 'src/lifecycle.rs'), 'utf8'),
      readFile(join(desktop, 'src/updates.rs'), 'utf8'),
      readFile(join(desktop, 'src/update_download.rs'), 'utf8'),
    ]);
  return verifyLifecycleSourceText({
    commandsSource,
    mainSource,
    buildSource,
    capability: JSON.parse(capabilityText),
    lifecycleSource,
    updaterSources: [updates, download],
  });
}

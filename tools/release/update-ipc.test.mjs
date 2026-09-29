import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { REPOSITORY_ROOT } from './release-common.mjs';

const desktop = (path) => join(REPOSITORY_ROOT, 'apps/desktop/src-tauri', path);

function parameters(signature) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < signature.length; index += 1) {
    const character = signature[index];
    if (character === '<') depth += 1;
    if (character === '>') depth -= 1;
    if (character === ',' && depth === 0) {
      const part = signature.slice(start, index).replace(/\s+/g, '');
      if (part) parts.push(part);
      start = index + 1;
    }
  }
  const last = signature.slice(start).replace(/\s+/g, '');
  if (last) parts.push(last);
  assert.equal(depth, 0, 'command parameter generics must be balanced');
  return parts;
}

test('manual update IPC accepts only Tauri-injected app and state', async () => {
  const source = await readFile(desktop('src/commands.rs'), 'utf8');
  const signatures = [
    ...source.matchAll(
      /#\[tauri::command\]\s*pub async fn check_for_update\s*\(([^)]*)\)\s*->/g,
    ),
  ];
  assert.equal(signatures.length, 1, 'exactly one update command is required');
  assert.deepEqual(parameters(signatures[0][1]), [
    'app:tauri::AppHandle',
    "service:State<'_,UpdateCheckService>",
  ]);
});

test('manual update IPC is registered in the app and permission generator', async () => {
  const [entry, manifest] = await Promise.all([
    readFile(desktop('src/main.rs'), 'utf8'),
    readFile(desktop('build.rs'), 'utf8'),
  ]);
  assert.equal([...entry.matchAll(/commands::check_for_update/g)].length, 1);
  assert.equal([...manifest.matchAll(/"check_for_update"/g)].length, 1);
});

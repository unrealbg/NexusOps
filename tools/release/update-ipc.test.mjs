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

test('update IPC exposes only hydration, check and opaque announcement download', async () => {
  const source = await readFile(desktop('src/commands.rs'), 'utf8');
  const signature = (name, kind = 'async ') => {
    const match = new RegExp(
      `#\\[tauri::command\\]\\s*pub ${kind}fn ${name}\\s*\\(([^)]*)\\)\\s*->`,
    ).exec(source);
    assert.ok(match, `${name} command must exist exactly once`);
    assert.equal(source.match(new RegExp(`pub ${kind}fn ${name}\\b`, 'g'))?.length, 1);
    return parameters(match[1]);
  };
  assert.deepEqual(signature('get_update_state', ''), [
    "lifecycle:State<'_,LifecycleCoordinator>",
    'app:tauri::AppHandle',
    "service:State<'_,UpdateService>",
  ]);
  assert.deepEqual(signature('check_for_update'), [
    "lifecycle:State<'_,LifecycleCoordinator>",
    'app:tauri::AppHandle',
    "service:State<'_,UpdateService>",
  ]);
  assert.deepEqual(signature('download_announced_update'), [
    "lifecycle:State<'_,LifecycleCoordinator>",
    'app:tauri::AppHandle',
    "service:State<'_,UpdateService>",
    'announcement_id:UpdateAnnouncementId',
  ]);
});

test('manual update IPC is registered in the app and permission generator', async () => {
  const [entry, manifest] = await Promise.all([
    readFile(desktop('src/main.rs'), 'utf8'),
    readFile(desktop('build.rs'), 'utf8'),
  ]);
  for (const command of ['get_update_state', 'check_for_update', 'download_announced_update']) {
    assert.equal([...entry.matchAll(new RegExp(`commands::${command}`, 'g'))].length, 1);
    assert.equal([...manifest.matchAll(new RegExp(`"${command}"`, 'g'))].length, 1);
  }
});

import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { invoke } from '@tauri-apps/api/core';
import { describe, expect, it, vi } from 'vitest';
import { updateApi } from './client';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: () => true,
}));

function frontendSource(directory: string): string {
  return readdirSync(directory, { withFileTypes: true })
    .map((entry) => {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) return frontendSource(path);
      if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) return '';
      return readFileSync(path, 'utf8');
    })
    .join('\n');
}

describe('update IPC boundary', () => {
  it('invokes only the argument-free NexusOps command', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      currentVersion: '0.1.0', status: 'upToDate', availableVersion: null,
    });
    expect(updateApi.check.length).toBe(0);
    await updateApi.check();
    expect(invoke).toHaveBeenCalledExactlyOnceWith('check_for_update', undefined);
  });

  it('has no direct updater plugin invocation or package import in runtime frontend source', () => {
    const repositorySource = join(process.cwd(), 'apps', 'desktop', 'src');
    const source = frontendSource(existsSync(repositorySource) ? repositorySource : join(process.cwd(), 'src'));
    expect(source).not.toContain(['plugin', 'updater'].join(':') + '|');
    expect(source).not.toContain(['@tauri-apps', 'plugin-updater'].join('/'));
  });

  it('does not schedule a check from the update control', () => {
    const component = join(process.cwd(), 'apps', 'desktop', 'src', 'components', 'UpdateCheck.tsx');
    const source = readFileSync(
      existsSync(component) ? component : join(process.cwd(), 'src', 'components', 'UpdateCheck.tsx'),
      'utf8',
    );
    expect(source).not.toMatch(/\b(?:useEffect|setInterval|setTimeout)\b/);
  });
});

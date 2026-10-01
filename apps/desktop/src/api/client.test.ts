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
  it('uses no-argument state/check commands and one opaque ID for each explicit authority', async () => {
    vi.mocked(invoke).mockResolvedValue({
      currentVersion: '0.1.0',
      phase: 'upToDate',
      availableVersion: null,
      announcementId: null,
    });
    expect(updateApi.state.length).toBe(0);
    expect(updateApi.check.length).toBe(0);
    expect(updateApi.download.length).toBe(1);
    expect(updateApi.install.length).toBe(1);
    await updateApi.state();
    await updateApi.check();
    await updateApi.download('opaque-announcement');
    await updateApi.install('opaque-verified-artifact');
    expect(invoke).toHaveBeenNthCalledWith(1, 'get_update_state', undefined);
    expect(invoke).toHaveBeenNthCalledWith(2, 'check_for_update', undefined);
    expect(invoke).toHaveBeenNthCalledWith(3, 'download_announced_update', {
      announcementId: 'opaque-announcement',
    });
    expect(invoke).toHaveBeenNthCalledWith(4, 'install_verified_update', {
      verifiedArtifactId: 'opaque-verified-artifact',
    });
  });

  it('has no direct updater plugin invocation or package import in runtime frontend source', () => {
    const repositorySource = join(process.cwd(), 'apps', 'desktop', 'src');
    const source = frontendSource(
      existsSync(repositorySource) ? repositorySource : join(process.cwd(), 'src'),
    );
    expect(source).not.toContain(['plugin', 'updater'].join(':') + '|');
    expect(source).not.toContain(['@tauri-apps', 'plugin-updater'].join('/'));
  });

  it('hydrates state once without scheduling checks or polling', () => {
    const component = join(
      process.cwd(),
      'apps',
      'desktop',
      'src',
      'components',
      'UpdateCheck.tsx',
    );
    const source = readFileSync(
      existsSync(component)
        ? component
        : join(process.cwd(), 'src', 'components', 'UpdateCheck.tsx'),
      'utf8',
    );
    expect(source.match(/useEffect\(\(\) =>/g)).toHaveLength(1);
    expect(source).toContain('updateApi.state()');
    expect(source).not.toMatch(/\b(?:setInterval|setTimeout)\b/);
    const hydration = /useEffect\(\(\) => \{([^]*?)\n\s{2}\}, \[\]\);/.exec(source);
    expect(hydration?.[1]).toContain('updateApi.state()');
    expect(hydration?.[1]).not.toContain('updateApi.check');
    expect(hydration?.[1]).not.toContain('updateApi.download');
  });
});

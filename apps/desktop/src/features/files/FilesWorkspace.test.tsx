import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DirectoryListing, SftpSessionInfo } from '@nexusops/protocol';
import { hostApi, filesApi } from '../../api/client';
import { connected, host } from '../../test/fixtures';
import { FilesWorkspace } from './FilesWorkspace';

vi.mock('../../api/client', async (original) => ({
  ...(await original<typeof import('../../api/client')>()),
  hostApi: { session: vi.fn() },
  filesApi: {
    open: vi.fn(), list: vi.fn(), properties: vi.fn(), chooseUploadFiles: vi.fn(),
    chooseDownloadDirectory: vi.fn(), planUpload: vi.fn(), planDownload: vi.fn(),
    planCreateDirectory: vi.fn(), planRename: vi.fn(), planDelete: vi.fn(),
    execute: vi.fn(), transfers: vi.fn(), cancel: vi.fn(), planRetry: vi.fn(),
  },
}));

const session: SftpSessionInfo = {
  id: 'sftp-1', hostId: host.id, hostSessionId: 'connection-1', protocolVersion: 3,
  extensions: [{ name: 'posix-rename@openssh.com', version: '1' }],
  limits: { maxPacketBytes: '262144', maxReadBytes: '65536', maxWriteBytes: '65536', maxOpenHandles: '64', clientChunkBytes: '65536', listingEntryCap: 5000 },
  rootPath: '/home/test',
};

function listing(path: string): DirectoryListing {
  return {
    hostId: host.id, hostSessionId: session.hostSessionId, sftpSessionId: session.id,
    path, partial: false, entryCap: 5000,
    entries: [{ name: 'данни.bin', displayName: 'данни.bin', path: `${path}/данни.bin`, kind: 'file', sizeBytes: '9007199254740993', modifiedAt: null, permissions: '100600', uid: 1000, gid: 1000 }],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((accept) => { resolve = accept; });
  return { promise, resolve };
}

function renderFiles(visible = true) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const result = render(<QueryClientProvider client={client}><FilesWorkspace host={host} visible={visible} onShowOverview={vi.fn()} /></QueryClientProvider>);
  return { ...result, client };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(hostApi.session).mockResolvedValue(connected);
  vi.mocked(filesApi.open).mockResolvedValue(session);
  vi.mocked(filesApi.list).mockImplementation(async (_, path) => listing(path));
  vi.mocked(filesApi.transfers).mockResolvedValue([]);
});

describe('Files workspace boundaries', () => {
  it('keeps the picker target bound to the directory visible when selection began', async () => {
    const picker = deferred<{ id: string; kind: 'uploadFiles'; items: []; expiresAt: string } | null>();
    vi.mocked(filesApi.chooseUploadFiles).mockReturnValue(picker.promise);
    vi.mocked(filesApi.planUpload).mockResolvedValue({
      id: 'plan-1', hostId: host.id, hostSessionId: session.hostSessionId,
      sftpSessionId: session.id, kind: 'upload', risk: 'moderate', conflictPolicy: 'skip',
      items: [], expiresAt: '2026-09-15T20:00:00Z',
    });
    renderFiles();
    expect(await screen.findByRole('button', { name: /данни\.bin/ })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    const address = screen.getByLabelText('Remote path');
    await userEvent.clear(address);
    await userEvent.type(address, '/home/other{Enter}');
    await screen.findByDisplayValue('/home/other');
    picker.resolve({ id: 'grant-1', kind: 'uploadFiles', items: [], expiresAt: '2026-09-15T20:00:00Z' });
    await waitFor(() => expect(filesApi.planUpload).toHaveBeenCalledWith(session, 'grant-1', '/home/test', 'skip'));
  });

  it('ignores an older delayed directory response and keeps lossless progress text', async () => {
    const slow = deferred<DirectoryListing>();
    vi.mocked(filesApi.list).mockImplementation(async (_, path) => path === '/slow' ? slow.promise : listing(path));
    vi.mocked(filesApi.transfers).mockResolvedValue([{
      id: 'job-1', hostId: host.id, hostSessionId: session.hostSessionId, sftpSessionId: session.id,
      direction: 'download', sourceDisplay: 'large.bin', destinationDisplay: 'large.bin',
      state: 'transferring', confirmedBytes: '9007199254740993', totalBytes: '18014398509481986', error: null, retryable: false,
    }]);
    renderFiles();
    await screen.findByDisplayValue('/home/test');
    const address = screen.getByLabelText('Remote path');
    await userEvent.clear(address); await userEvent.type(address, '/slow{Enter}');
    await userEvent.clear(address); await userEvent.type(address, '/newest{Enter}');
    expect(await screen.findByDisplayValue('/newest')).toBeInTheDocument();
    slow.resolve(listing('/slow'));
    await waitFor(() => expect(screen.getByDisplayValue('/newest')).toBeInTheDocument());
    expect(await screen.findByText('9007199254740993 / 18014398509481986 bytes (50%)')).toBeInTheDocument();
  });

  it('does not cancel backend transfers when the workspace unmounts', async () => {
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    view.rerender(<QueryClientProvider client={view.client}><FilesWorkspace host={host} visible={false} onShowOverview={vi.fn()} /></QueryClientProvider>);
    expect(filesApi.cancel).not.toHaveBeenCalled();
  });
});

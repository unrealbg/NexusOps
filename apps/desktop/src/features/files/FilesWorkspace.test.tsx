import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DirectoryListing, FileOperationPlan, SftpSessionInfo } from '@nexusops/protocol';
import { hostApi, filesApi } from '../../api/client';
import { hostKeys } from '../../api/queries';
import { connected, disconnected, host } from '../../test/fixtures';
import { FilesWorkspace } from './FilesWorkspace';

vi.mock('../../api/client', async (original) => ({
  ...(await original<typeof import('../../api/client')>()),
  hostApi: { session: vi.fn() },
  filesApi: {
    open: vi.fn(), list: vi.fn(), properties: vi.fn(), chooseUploadFiles: vi.fn(),
    openText: vi.fn(), planTextSave: vi.fn(),
    chooseDownloadDirectory: vi.fn(), planUpload: vi.fn(), planDownload: vi.fn(),
    planCreateDirectory: vi.fn(), planRename: vi.fn(), planDelete: vi.fn(),
    execute: vi.fn(), discardPlan: vi.fn(), discardGrant: vi.fn(),
    transfers: vi.fn(), cancel: vi.fn(), planRetry: vi.fn(),
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

function uploadPlan(id = 'plan-upload'): FileOperationPlan {
  return {
    id, hostId: host.id, hostSessionId: session.hostSessionId, sftpSessionId: session.id,
    kind: 'upload', risk: 'moderate', conflictPolicy: 'skip', items: [], expiresAt: '2026-10-01T20:00:00Z',
  };
}

const nextSession: SftpSessionInfo = { ...session, id: 'sftp-2', hostSessionId: 'connection-2' };

const uploadGrant = { id: 'grant-upload', kind: 'uploadFiles' as const, items: [], expiresAt: '2026-10-01T20:00:00Z' };

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
  it('enables Edit only for one selected regular file', async () => {
    vi.mocked(filesApi.list).mockImplementation(async (_, path) => ({
      ...listing(path), entries: [
        ...listing(path).entries,
        { name: 'folder', displayName: 'folder', path: `${path}/folder`, kind: 'directory', sizeBytes: null, modifiedAt: null, permissions: null, uid: null, gid: null },
        { name: 'link', displayName: 'link', path: `${path}/link`, kind: 'symlink', sizeBytes: null, modifiedAt: null, permissions: null, uid: null, gid: null },
      ],
    }));
    renderFiles();
    const edit = await screen.findByRole('button', { name: 'Edit' });
    expect(edit).toBeDisabled();
    await screen.findByRole('checkbox', { name: 'Select folder' });
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select folder' }));
    expect(edit).toBeDisabled();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select folder' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select link' }));
    expect(edit).toBeDisabled();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select link' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select данни.bin' }));
    expect(edit).toBeEnabled();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select folder' }));
    expect(edit).toBeDisabled();
  });

  it('keeps a dirty editor mounted while Files is hidden and blocks replacing it', async () => {
    vi.mocked(filesApi.openText).mockResolvedValue({
      id: 'document-a', hostId: host.id, hostSessionId: session.hostSessionId, sftpSessionId: session.id,
      path: '/home/test/данни.bin', text: 'old', originalBytes: 3, newline: 'lf', bom: false, maxBytes: 1048576,
    });
    const result = renderFiles();
    await screen.findByRole('checkbox', { name: 'Select данни.bin' });
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select данни.bin' }));
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const editor = await screen.findByRole('textbox', { name: 'Remote text' });
    await userEvent.type(editor, 'new');
    expect(screen.getByRole('button', { name: 'Edit' })).toBeDisabled();
    result.rerender(<QueryClientProvider client={result.client}><FilesWorkspace host={host} visible={false} onShowOverview={vi.fn()} /></QueryClientProvider>);
    expect(screen.queryByRole('textbox', { name: 'Remote text' })).not.toBeInTheDocument();
    result.rerender(<QueryClientProvider client={result.client}><FilesWorkspace host={host} visible onShowOverview={vi.fn()} /></QueryClientProvider>);
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('oldnew');
    expect(filesApi.openText).toHaveBeenCalledOnce();
  });
  it('does not surface a delayed upload plan after disconnect', async () => {
    const planning = deferred<FileOperationPlan>();
    vi.mocked(filesApi.chooseUploadFiles).mockResolvedValue(uploadGrant);
    vi.mocked(filesApi.planUpload).mockReturnValue(planning.promise);
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    await waitFor(() => expect(filesApi.planUpload).toHaveBeenCalledOnce());
    await act(async () => { view.client.setQueryData(hostKeys.session(host.id), disconnected); });
    await screen.findByText(/Connect this host to browse files/);
    await act(async () => { planning.resolve(uploadPlan()); await planning.promise; });
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
    expect(filesApi.discardPlan).toHaveBeenCalledWith(session, 'plan-upload');
  });

  it('does not rebind an old upload approval to a new SFTP session', async () => {
    const planning = deferred<FileOperationPlan>();
    vi.mocked(filesApi.open).mockResolvedValueOnce(session).mockResolvedValueOnce(nextSession);
    vi.mocked(filesApi.chooseUploadFiles).mockResolvedValue(uploadGrant);
    vi.mocked(filesApi.planUpload).mockReturnValue(planning.promise);
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    await waitFor(() => expect(filesApi.planUpload).toHaveBeenCalledOnce());
    await act(async () => { view.client.setQueryData(hostKeys.session(host.id), disconnected); });
    await screen.findByText(/Connect this host to browse files/);
    await act(async () => { view.client.setQueryData(hostKeys.session(host.id), connected); });
    await waitFor(() => expect(filesApi.list).toHaveBeenCalledWith(nextSession, '/home/test'));
    await act(async () => { planning.resolve(uploadPlan()); await planning.promise; });
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
    expect(filesApi.discardPlan).toHaveBeenCalledWith(session, 'plan-upload');
    expect(filesApi.discardPlan).not.toHaveBeenCalledWith(nextSession, 'plan-upload');
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('ignores an old planning response when a visible reopen replaces SFTP identity', async () => {
    const planning = deferred<FileOperationPlan>();
    vi.mocked(filesApi.open).mockResolvedValueOnce(session).mockResolvedValueOnce(nextSession);
    vi.mocked(filesApi.chooseUploadFiles).mockResolvedValue(uploadGrant);
    vi.mocked(filesApi.planUpload).mockReturnValue(planning.promise);
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    await waitFor(() => expect(filesApi.planUpload).toHaveBeenCalledOnce());
    view.rerender(<QueryClientProvider client={view.client}><FilesWorkspace host={host} visible={false} onShowOverview={vi.fn()} /></QueryClientProvider>);
    view.rerender(<QueryClientProvider client={view.client}><FilesWorkspace host={host} visible onShowOverview={vi.fn()} /></QueryClientProvider>);
    await waitFor(() => expect(filesApi.list).toHaveBeenCalledWith(nextSession, '/home/test'));
    await act(async () => { planning.resolve(uploadPlan()); await planning.promise; });
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
    expect(filesApi.discardPlan).toHaveBeenCalledWith(session, 'plan-upload');
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('ignores a delayed download plan after disconnect and discards under its old owner', async () => {
    const planning = deferred<FileOperationPlan>();
    vi.mocked(filesApi.chooseDownloadDirectory).mockResolvedValue({
      id: 'grant-download', kind: 'downloadDirectory', items: [], expiresAt: '2026-10-01T20:00:00Z',
    });
    vi.mocked(filesApi.planDownload).mockReturnValue(planning.promise);
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select данни.bin' }));
    await userEvent.click(screen.getByRole('button', { name: 'Download selected…' }));
    await waitFor(() => expect(filesApi.planDownload).toHaveBeenCalledOnce());
    await act(async () => { view.client.setQueryData(hostKeys.session(host.id), disconnected); });
    await screen.findByText(/Connect this host to browse files/);
    const plan = { ...uploadPlan('plan-download-stale'), kind: 'download' as const };
    await act(async () => { planning.resolve(plan); await planning.promise; });
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
    expect(filesApi.discardPlan).toHaveBeenCalledWith(session, 'plan-download-stale');
  });

  it('removes an already visible approval when the host disconnects', async () => {
    vi.mocked(filesApi.chooseUploadFiles).mockResolvedValue(uploadGrant);
    vi.mocked(filesApi.planUpload).mockResolvedValue(uploadPlan());
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    expect(await screen.findByRole('dialog', { name: 'Approve file operation' })).toBeInTheDocument();
    await act(async () => { view.client.setQueryData(hostKeys.session(host.id), disconnected); });
    await screen.findByText(/Connect this host to browse files/);
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('executes a current approval once and removes it before a second click', async () => {
    vi.mocked(filesApi.chooseUploadFiles).mockResolvedValue(uploadGrant);
    vi.mocked(filesApi.planUpload).mockResolvedValue(uploadPlan());
    vi.mocked(filesApi.execute).mockResolvedValue([]);
    renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    const approve = await screen.findByRole('button', { name: 'Approve and execute' });
    await act(async () => { fireEvent.click(approve); fireEvent.click(approve); });
    await waitFor(() => expect(filesApi.execute).toHaveBeenCalledTimes(1));
    expect(filesApi.execute).toHaveBeenCalledWith(session, 'plan-upload');
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
  });

  it('handles an in-flight Approve rejection after disconnect without retrying', async () => {
    let rejectExecute!: (reason: Error) => void;
    vi.mocked(filesApi.chooseUploadFiles).mockResolvedValue(uploadGrant);
    vi.mocked(filesApi.planUpload).mockResolvedValue(uploadPlan());
    vi.mocked(filesApi.execute).mockReturnValue(new Promise((_, reject) => { rejectExecute = reject; }));
    vi.mocked(filesApi.discardPlan).mockResolvedValue(undefined);
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Approve and execute' }));
    expect(filesApi.execute).toHaveBeenCalledTimes(1);
    await act(async () => { view.client.setQueryData(hostKeys.session(host.id), disconnected); });
    await screen.findByText(/Connect this host to browse files/);
    await act(async () => { rejectExecute(new Error('SFTP closed')); });
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
    expect(filesApi.execute).toHaveBeenCalledTimes(1);
  });

  it('drops a visible old approval when SFTP identity changes on reopen', async () => {
    vi.mocked(filesApi.open).mockResolvedValueOnce(session).mockResolvedValueOnce(nextSession);
    vi.mocked(filesApi.chooseUploadFiles).mockResolvedValue(uploadGrant);
    vi.mocked(filesApi.planUpload).mockResolvedValue(uploadPlan());
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    expect(await screen.findByRole('dialog', { name: 'Approve file operation' })).toBeInTheDocument();
    view.rerender(<QueryClientProvider client={view.client}><FilesWorkspace host={host} visible={false} onShowOverview={vi.fn()} /></QueryClientProvider>);
    view.rerender(<QueryClientProvider client={view.client}><FilesWorkspace host={host} visible onShowOverview={vi.fn()} /></QueryClientProvider>);
    await waitFor(() => expect(filesApi.list).toHaveBeenCalledWith(nextSession, '/home/test'));
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('ignores a delayed create-directory plan after disconnect', async () => {
    const planning = deferred<FileOperationPlan>();
    vi.mocked(filesApi.planCreateDirectory).mockReturnValue(planning.promise);
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'New directory' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Name' }), 'new-directory');
    await userEvent.click(screen.getByRole('button', { name: 'Review plan' }));
    await waitFor(() => expect(filesApi.planCreateDirectory).toHaveBeenCalledOnce());
    await act(async () => { view.client.setQueryData(hostKeys.session(host.id), disconnected); });
    await screen.findByText(/Connect this host to browse files/);
    const plan = { ...uploadPlan('plan-create-stale'), kind: 'createDirectory' as const, conflictPolicy: null };
    await act(async () => { planning.resolve(plan); await planning.promise; });
    expect(screen.queryByRole('dialog', { name: 'Approve file operation' })).not.toBeInTheDocument();
    expect(filesApi.discardPlan).toHaveBeenCalledWith(session, 'plan-create-stale');
  });

  it('handles a rejected Cancel around turnover without double-discard or an unhandled rejection', async () => {
    vi.mocked(filesApi.chooseUploadFiles).mockResolvedValue(uploadGrant);
    vi.mocked(filesApi.planUpload).mockResolvedValue(uploadPlan());
    vi.mocked(filesApi.discardPlan).mockRejectedValue(new Error('SFTP closed'));
    const view = renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('button', { name: 'Upload files…' }));
    const close = await screen.findByRole('button', { name: 'Close dialog' });
    await act(async () => {
      fireEvent.click(close);
      fireEvent.click(close);
      view.client.setQueryData(hostKeys.session(host.id), disconnected);
    });
    await screen.findByText(/Connect this host to browse files/);
    expect(filesApi.discardPlan).toHaveBeenCalledTimes(1);
    expect(filesApi.discardPlan).toHaveBeenCalledWith(session, 'plan-upload');
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

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
    expect(filesApi.chooseUploadFiles).toHaveBeenCalledWith(session);
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

  it('shows the full final download target and discards a cancelled approval', async () => {
    vi.mocked(filesApi.chooseDownloadDirectory).mockResolvedValue({
      id: 'grant-download', kind: 'downloadDirectory', items: [], expiresAt: '2026-09-15T20:00:00Z',
    });
    vi.mocked(filesApi.planDownload).mockResolvedValue({
      id: 'plan-download', hostId: host.id, hostSessionId: session.hostSessionId,
      sftpSessionId: session.id, kind: 'download', risk: 'moderate', conflictPolicy: 'keepBoth',
      items: [{
        sourceDisplay: 'данни.bin',
        destinationDisplay: 'C:\\Users\\owner\\Downloads\\данни (1).bin',
        sizeBytes: '9007199254740993',
      }],
      expiresAt: '2026-09-15T20:00:00Z',
    });
    vi.mocked(filesApi.discardPlan).mockResolvedValue(undefined);
    renderFiles();
    await screen.findByDisplayValue('/home/test');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Select данни.bin' }));
    await userEvent.selectOptions(screen.getByLabelText('Conflict'), 'keepBoth');
    await userEvent.click(screen.getByRole('button', { name: 'Download selected…' }));

    expect(await screen.findByText('C:\\Users\\owner\\Downloads\\данни (1).bin')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(filesApi.discardPlan).toHaveBeenCalledWith(session, 'plan-download'));
    expect(filesApi.discardPlan).toHaveBeenCalledTimes(1);
    expect(filesApi.execute).not.toHaveBeenCalled();
  });
});

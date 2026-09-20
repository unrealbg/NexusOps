import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { FileOperationPlan, RemoteTextDocument, SftpSessionInfo, TransferJob } from '@nexusops/protocol';
import { filesApi } from '../../api/client';
import { host } from '../../test/fixtures';
import { RemoteTextEditor } from './RemoteTextEditor';

vi.mock('../../api/client', async (original) => ({
  ...(await original<typeof import('../../api/client')>()),
  filesApi: { openText: vi.fn(), planTextSave: vi.fn(), execute: vi.fn(), discardPlan: vi.fn() },
}));

const session: SftpSessionInfo = {
  id: 'sftp-a', hostId: host.id, hostSessionId: 'connection-a', protocolVersion: 3,
  extensions: [], limits: { maxPacketBytes: null, maxReadBytes: null, maxWriteBytes: null,
    maxOpenHandles: null, clientChunkBytes: '65536', listingEntryCap: 5000 }, rootPath: '/home/test',
};
const document: RemoteTextDocument = {
  id: 'document-a', hostId: host.id, hostSessionId: session.hostSessionId, sftpSessionId: session.id,
  path: '/home/test/данни.txt', text: 'old\n', originalBytes: 4, newline: 'lf', bom: false, maxBytes: 1048576,
};
const plan: FileOperationPlan = {
  id: 'plan-a', hostId: host.id, hostSessionId: session.hostSessionId, sftpSessionId: session.id,
  kind: 'editText', risk: 'high', conflictPolicy: null,
  items: [{ sourceDisplay: 'Edited text', destinationDisplay: document.path, sizeBytes: '4' }],
  expiresAt: '2026-10-01T20:00:00Z',
};
function job(state: TransferJob['state']): TransferJob {
  return { id: 'job-a', hostId: host.id, hostSessionId: session.hostSessionId, sftpSessionId: session.id,
    direction: 'upload', sourceDisplay: 'Edited text', destinationDisplay: document.path,
    state, confirmedBytes: '4', totalBytes: '4', error: null, retryable: false };
}
function view(overrides: Partial<React.ComponentProps<typeof RemoteTextEditor>> = {}) {
  const props = { initial: document, host, session, connected: true, visible: true, transfers: [] as TransferJob[], onClose: vi.fn(), ...overrides };
  return { ...render(<RemoteTextEditor {...props} />), props };
}
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(filesApi.planTextSave).mockResolvedValue(plan);
  vi.mocked(filesApi.discardPlan).mockResolvedValue();
  vi.mocked(filesApi.execute).mockResolvedValue([job('queued')]);
  vi.mocked(filesApi.openText).mockResolvedValue(document);
});

describe('remote text editor authority and interaction', () => {
  it('reviews physical KeyS exactly once, including BG layout, but ignores AltGr', async () => {
    view();
    const editor = screen.getByRole('textbox', { name: 'Remote text' });
    await userEvent.clear(editor);
    await userEvent.type(editor, 'new');
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument();
    fireEvent.keyDown(editor, { key: 'ы', code: 'KeyS', ctrlKey: true, altKey: true });
    expect(filesApi.planTextSave).not.toHaveBeenCalled();
    fireEvent.keyDown(editor, { key: 'ы', code: 'KeyS', ctrlKey: true });
    fireEvent.keyDown(editor, { key: 'ы', code: 'KeyS', ctrlKey: true });
    expect(filesApi.planTextSave).toHaveBeenCalledOnce();
    expect(filesApi.planTextSave).toHaveBeenCalledWith(session, document.id, 'new');
    expect(await screen.findByRole('dialog', { name: 'Approve remote text save' })).toBeInTheDocument();
    expect(screen.getByText('Risk: high. Original: 4 bytes. New: 4 bytes.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Approve and save' }));
    await waitFor(() => expect(filesApi.execute).toHaveBeenCalledOnce());
  });

  it('discards a current approval exactly once and requires reload before another save', async () => {
    view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'new');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(filesApi.discardPlan).toHaveBeenCalledExactlyOnceWith(session, plan.id);
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
  });

  it('keeps dirty text across disconnect and never rebinds it to a new session', async () => {
    const result = view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'new');
    result.rerender(<RemoteTextEditor {...result.props} connected={false} session={null} />);
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('old\nnew');
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
    result.rerender(<RemoteTextEditor {...result.props} session={{ ...session, id: 'sftp-b', hostSessionId: 'connection-b' }} />);
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('old\nnew');
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('drops a delayed old plan and retains text after turnover', async () => {
    let resolve!: (value: FileOperationPlan) => void;
    vi.mocked(filesApi.planTextSave).mockReturnValue(new Promise((done) => { resolve = done; }));
    const result = view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'new');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    result.rerender(<RemoteTextEditor {...result.props} connected={false} session={null} />);
    await act(async () => { resolve(plan); });
    expect(screen.queryByRole('dialog', { name: 'Approve remote text save' })).not.toBeInTheDocument();
    expect(filesApi.discardPlan).toHaveBeenCalledWith(session, plan.id);
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('old\nnew');
  });

  it('retains local text after conflict and confirms dirty close and reload', async () => {
    vi.mocked(filesApi.planTextSave).mockRejectedValue({ code: 'conflict', message: 'Changed remotely' });
    const result = view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'new');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByText('Changed remotely');
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('old\nnew');
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.getByRole('dialog', { name: 'Discard unsaved changes?' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Keep editing' }));
    expect(result.props.onClose).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Reload remote' }));
    expect(screen.getByRole('dialog', { name: 'Reload remote text?' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Discard changes' }));
    await waitFor(() => expect(filesApi.openText).toHaveBeenCalledWith(session, document.path));
  });

  it('clears dirty state only after confirmed transfer completion', async () => {
    const result = view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'new');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    await userEvent.click(screen.getByRole('button', { name: 'Approve and save' }));
    await waitFor(() => expect(filesApi.execute).toHaveBeenCalledOnce());
    result.rerender(<RemoteTextEditor {...result.props} transfers={[job('completed')]} />);
    await waitFor(() => expect(screen.getByText('No unsaved changes')).toBeInTheDocument());
  });
});

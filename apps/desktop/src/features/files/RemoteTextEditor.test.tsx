import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { FileOperationPlan, RemoteTextDocument, SftpSessionInfo, TransferJob } from '@nexusops/protocol';
import { filesApi } from '../../api/client';
import { host } from '../../test/fixtures';
import { RemoteTextEditor } from './RemoteTextEditor';

vi.mock('../../api/client', async (original) => ({
  ...(await original<typeof import('../../api/client')>()),
  filesApi: { openText: vi.fn(), planTextSave: vi.fn(), execute: vi.fn(), discardPlan: vi.fn(), discardEditorPlan: vi.fn(), discardTextDocument: vi.fn() },
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
  vi.resetAllMocks();
  vi.mocked(filesApi.planTextSave).mockResolvedValue(plan);
  vi.mocked(filesApi.discardPlan).mockResolvedValue();
  vi.mocked(filesApi.discardEditorPlan).mockResolvedValue(true);
  vi.mocked(filesApi.discardTextDocument).mockResolvedValue();
  vi.mocked(filesApi.execute).mockResolvedValue([job('queued')]);
  vi.mocked(filesApi.openText).mockResolvedValue(document);
});

describe('remote text editor authority and interaction', () => {
  it('ED-01 discards a delayed plan under its original session after confirmed close and unmount', async () => {
    let resolve!: (value: FileOperationPlan) => void;
    vi.mocked(filesApi.planTextSave).mockReturnValue(new Promise((done) => { resolve = done; }));
    const result = view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'A');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    await userEvent.click(screen.getByRole('button', { name: 'Discard changes' }));
    expect(result.props.onClose).toHaveBeenCalledOnce();
    result.unmount();
    await act(async () => { resolve(plan); });
    expect(filesApi.discardPlan).toHaveBeenCalledExactlyOnceWith(session, plan.id);
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('ED-02 discards approval for an older buffer snapshot while retaining newer text', async () => {
    let resolve!: (value: FileOperationPlan) => void;
    vi.mocked(filesApi.planTextSave).mockReturnValue(new Promise((done) => { resolve = done; }));
    view();
    const editor = screen.getByRole('textbox', { name: 'Remote text' });
    fireEvent.change(editor, { target: { value: 'text A' } });
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    fireEvent.change(editor, { target: { value: 'text B' } });
    await act(async () => { resolve(plan); });
    expect(editor).toHaveValue('text B');
    expect(screen.queryByRole('dialog', { name: 'Approve remote text save' })).not.toBeInTheDocument();
    expect(filesApi.discardPlan).toHaveBeenCalledExactlyOnceWith(session, plan.id);
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('ED-04 handles physical Ctrl+S from an editor control exactly once', async () => {
    view();
    fireEvent.change(screen.getByRole('textbox', { name: 'Remote text' }), { target: { value: 'changed' } });
    const control = screen.getByRole('button', { name: 'Close' });
    control.focus();
    fireEvent.keyDown(control, { key: 'ы', code: 'KeyS', ctrlKey: true });
    fireEvent.keyDown(control, { key: 'ы', code: 'KeyS', ctrlKey: true });
    expect(filesApi.planTextSave).toHaveBeenCalledOnce();
  });

  it('handles English KeyS in the textarea but not a keydown outside the editor', () => {
    view();
    const editor = screen.getByRole('textbox', { name: 'Remote text' });
    fireEvent.change(editor, { target: { value: 'changed' } });
    expect(fireEvent.keyDown(globalThis.document.body, { key: 's', code: 'KeyS', ctrlKey: true })).toBe(true);
    expect(filesApi.planTextSave).not.toHaveBeenCalled();
    expect(fireEvent.keyDown(editor, { key: 's', code: 'KeyS', ctrlKey: true })).toBe(false);
    expect(filesApi.planTextSave).toHaveBeenCalledOnce();
  });

  it('ED-04 lets AltGraph pass through even when Ctrl is reported', () => {
    view();
    const editor = screen.getByRole('textbox', { name: 'Remote text' });
    fireEvent.change(editor, { target: { value: 'changed' } });
    const key = new KeyboardEvent('keydown', { key: 's', code: 'KeyS', ctrlKey: true, bubbles: true, cancelable: true });
    Object.defineProperty(key, 'getModifierState', { value: (modifier: string) => modifier === 'AltGraph' });
    const handled = fireEvent(editor, key);
    expect(handled).toBe(true);
    expect(filesApi.planTextSave).not.toHaveBeenCalled();
  });

  it('passes unsupported modifiers, repeat and confirmation through without save review', async () => {
    view();
    const editor = screen.getByRole('textbox', { name: 'Remote text' });
    fireEvent.change(editor, { target: { value: 'changed' } });
    for (const modifiers of [{ altKey: true }, { metaKey: true }, { repeat: true }]) {
      expect(fireEvent.keyDown(editor, { key: 's', code: 'KeyS', ctrlKey: true, ...modifiers })).toBe(true);
    }
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(fireEvent.keyDown(editor, { key: 's', code: 'KeyS', ctrlKey: true })).toBe(true);
    expect(filesApi.planTextSave).not.toHaveBeenCalled();
  });

  it('retires a clean document on Close once, including unmount cleanup', async () => {
    const result = view();
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    result.unmount();
    await waitFor(() => expect(filesApi.discardTextDocument).toHaveBeenCalledExactlyOnceWith(
      { hostId: document.hostId, hostSessionId: document.hostSessionId, id: document.sftpSessionId }, document.id));
  });

  it('retires the old token after reload and the replacement on unmount', async () => {
    const replacement = { ...document, id: 'document-b', text: 'fresh' };
    vi.mocked(filesApi.openText).mockResolvedValueOnce(replacement);
    const result = view();
    await userEvent.click(screen.getByRole('button', { name: 'Reload remote' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('fresh'));
    expect(filesApi.discardTextDocument).toHaveBeenCalledWith(
      { hostId: document.hostId, hostSessionId: document.hostSessionId, id: document.sftpSessionId }, document.id);
    result.unmount();
    await waitFor(() => expect(filesApi.discardTextDocument).toHaveBeenCalledWith(
      { hostId: replacement.hostId, hostSessionId: replacement.hostSessionId, id: replacement.sftpSessionId }, replacement.id));
  });

  it('retires an ignored reload token returned after editor unmount', async () => {
    let resolve!: (value: RemoteTextDocument) => void;
    vi.mocked(filesApi.openText).mockReturnValue(new Promise((done) => { resolve = done; }));
    const result = view();
    await userEvent.click(screen.getByRole('button', { name: 'Reload remote' }));
    result.unmount();
    await act(async () => { resolve({ ...document, id: 'late-document' }); });
    expect(filesApi.discardTextDocument).toHaveBeenCalledWith(
      { hostId: document.hostId, hostSessionId: document.hostSessionId, id: document.sftpSessionId }, 'late-document');
  });

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

  it('discards P1 and permits a fresh P2 review of the unchanged dirty text without reload', async () => {
    const secondPlan = { ...plan, id: 'plan-b' };
    vi.mocked(filesApi.planTextSave).mockResolvedValueOnce(plan).mockResolvedValueOnce(secondPlan);
    view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'new');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(filesApi.discardEditorPlan).toHaveBeenCalledExactlyOnceWith(session, plan.id);
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('old\nnew');
    expect(screen.getByRole('status')).toHaveTextContent('Save cancelled. Your unsaved changes are retained.');
    expect(screen.queryByText(/Remote authority is stale or disconnected/)).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save / review' })).toBeEnabled());
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    expect(filesApi.planTextSave).toHaveBeenNthCalledWith(2, session, document.id, 'old\nnew');
    expect(await screen.findByRole('dialog', { name: 'Approve remote text save' })).toBeInTheDocument();
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('keeps review disabled until discard confirms and plans the latest buffer snapshot', async () => {
    let release!: (value: boolean) => void;
    vi.mocked(filesApi.discardEditorPlan).mockReturnValue(new Promise((done) => { release = done; }));
    vi.mocked(filesApi.planTextSave).mockResolvedValueOnce(plan).mockResolvedValueOnce({ ...plan, id: 'plan-b' });
    view();
    const editor = screen.getByRole('textbox', { name: 'Remote text' });
    await userEvent.type(editor, 'A');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    await userEvent.click(screen.getByRole('button', { name: 'Close dialog' }));
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Reload remote' })).toBeDisabled();
    expect(screen.getByRole('status')).toHaveTextContent('Cancelling save review');
    await act(async () => { release(true); });
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeEnabled();
    await userEvent.type(editor, 'B');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    expect(filesApi.planTextSave).toHaveBeenNthCalledWith(2, session, document.id, 'old\nAB');
    expect(await screen.findByRole('dialog', { name: 'Approve remote text save' })).toBeInTheDocument();
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('treats Escape as Cancel and blocks retry when backend discard is unconfirmed', async () => {
    vi.mocked(filesApi.discardEditorPlan).mockResolvedValue(false);
    view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'A');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    const dialog = await screen.findByRole('dialog', { name: 'Approve remote text save' });
    fireEvent(dialog, new Event('cancel', { bubbles: true, cancelable: true }));
    await screen.findByText(/save plan was not confirmed cancelled/i);
    expect(filesApi.discardEditorPlan).toHaveBeenCalledExactlyOnceWith(session, plan.id);
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('old\nA');
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
    expect(screen.queryByText('Save cancelled. Your unsaved changes are retained.')).not.toBeInTheDocument();
  });

  it('does not revive authority after disconnect during a delayed discard', async () => {
    let release!: (value: boolean) => void;
    vi.mocked(filesApi.discardEditorPlan).mockReturnValue(new Promise((done) => { release = done; }));
    const result = view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'A');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    result.rerender(<RemoteTextEditor {...result.props} connected={false} session={null} />);
    await act(async () => { release(true); });
    result.rerender(<RemoteTextEditor {...result.props} session={{ ...session, id: 'sftp-b', hostSessionId: 'connection-b' }} />);
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('old\nA');
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
    expect(screen.queryByText('Save cancelled. Your unsaved changes are retained.')).not.toBeInTheDocument();
  });

  it('clears a prior successful Cancel notice when the session disconnects', async () => {
    const result = view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'A');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(await screen.findByText('Save cancelled. Your unsaved changes are retained.')).toBeInTheDocument();
    result.rerender(<RemoteTextEditor {...result.props} connected={false} session={null} />);
    await waitFor(() => expect(screen.queryByText('Save cancelled. Your unsaved changes are retained.')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
  });

  it('does not revive a retired document when discard completes after dirty Close and unmount', async () => {
    let release!: (value: boolean) => void;
    vi.mocked(filesApi.discardEditorPlan).mockReturnValue(new Promise((done) => { release = done; }));
    const result = view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'A');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    await userEvent.click(screen.getByRole('button', { name: 'Discard changes' }));
    result.unmount();
    await act(async () => { release(true); });
    expect(result.props.onClose).toHaveBeenCalledOnce();
    expect(filesApi.discardTextDocument).toHaveBeenCalledExactlyOnceWith(
      { hostId: document.hostId, hostSessionId: document.hostSessionId, id: document.sftpSessionId }, document.id);
    expect(filesApi.execute).not.toHaveBeenCalled();
  });

  it('retains dirty text and blocks retry when discard rejects', async () => {
    vi.mocked(filesApi.discardEditorPlan).mockRejectedValue({ code: 'sftpUnavailable', message: 'Session closed during cancel' });
    view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'A');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await screen.findByText(/Save cancellation could not be confirmed/);
    expect(screen.getByRole('textbox', { name: 'Remote text' })).toHaveValue('old\nA');
    expect(screen.getByRole('button', { name: 'Save / review' })).toBeDisabled();
    expect(filesApi.planTextSave).toHaveBeenCalledOnce();
  });

  it('approves a plan once even if a second click races with Cancel', async () => {
    view();
    await userEvent.type(screen.getByRole('textbox', { name: 'Remote text' }), 'A');
    await userEvent.click(screen.getByRole('button', { name: 'Save / review' }));
    await screen.findByRole('dialog', { name: 'Approve remote text save' });
    const approve = screen.getByRole('button', { name: 'Approve and save' });
    const cancel = screen.getByRole('button', { name: 'Cancel' });
    await act(async () => { fireEvent.click(approve); fireEvent.click(cancel); fireEvent.click(approve); });
    expect(filesApi.execute).toHaveBeenCalledExactlyOnceWith(session, plan.id);
    expect(filesApi.discardEditorPlan).not.toHaveBeenCalled();
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

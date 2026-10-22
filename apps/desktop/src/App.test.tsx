import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Host, TerminalSession, UpdateOperationSnapshot } from '@nexusops/protocol';
import App from './App';
import { containersApi, hostApi, securityApi, logsApi, updateApi, terminalApi, filesApi, monitorApi } from './api/client';
import { useSelection } from './state/selection';
import { connected, disconnected, host } from './test/fixtures';

// Real workspace orchestration with an inert xterm boundary. TerminalWorkspace's
// own tests exercise the real TerminalPane against the established xterm mocks.
vi.mock('./features/terminal/TerminalPane', () => ({
  TerminalPane: ({ session, active, visible }: { session: TerminalSession; active: boolean; visible: boolean }) =>
    <div hidden={!active} data-testid={session.id} data-visible={visible}>
      <textarea aria-label={`Synthetic ${session.label}`} defaultValue="Synthetic local scrollback" />
    </div>,
}));

vi.mock('./api/client', async (original) => ({
  ...(await original<typeof import('./api/client')>()),
  securityApi: { get: vi.fn() },
  logsApi: { list: vi.fn() },
  containersApi: { list: vi.fn() },
  updateApi: { state: vi.fn(), check: vi.fn(), download: vi.fn(), install: vi.fn() },
  terminalApi: { list: vi.fn(), open: vi.fn(), close: vi.fn(), rename: vi.fn() },
  filesApi: { open: vi.fn(), list: vi.fn(), transfers: vi.fn() },
  monitorApi: { sample: vi.fn() },
  hostApi: {
    list: vi.fn(),
    save: vi.fn(),
    delete: vi.fn(),
    session: vi.fn(),
    connect: vi.fn(),
    disconnect: vi.fn(),
    reconnect: vi.fn(),
    trust: vi.fn(),
    refresh: vi.fn(),
  },
}));

function renderApp() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
  );
  return client;
}
beforeEach(() => {
  useSelection.setState({ hostId: null });
  vi.mocked(hostApi.list).mockResolvedValue([]);
  vi.mocked(hostApi.session).mockResolvedValue(disconnected);
  vi.mocked(terminalApi.list).mockReset().mockResolvedValue([]);
  vi.mocked(terminalApi.open).mockReset();
  vi.mocked(filesApi.open).mockReset();
  vi.mocked(filesApi.list).mockReset();
  vi.mocked(filesApi.transfers).mockReset().mockResolvedValue([]);
  vi.mocked(monitorApi.sample).mockReset().mockImplementation(async (hostId, hostSessionId) => ({
    hostId, hostSessionId, observedAt: new Date().toISOString(), cpuUsagePercent: null,
    memoryUsedBytes: null, memoryTotalBytes: null, rootUsedBytes: null, rootTotalBytes: null,
    swapUsedBytes: null, swapTotalBytes: null, networkRxBytesPerSecond: null, networkTxBytesPerSecond: null, warnings: [],
  }));
  vi.mocked(securityApi.get).mockResolvedValue({
    hostId: host.id,
    hostname: host.connection.hostname,
    port: host.connection.port,
    authentication: host.connection.authentication,
    endpointPin: null,
  });
  vi.mocked(containersApi.list).mockReset();
  vi.mocked(updateApi.state).mockReset();
  vi.mocked(updateApi.check).mockReset();
  vi.mocked(updateApi.download).mockReset();
  vi.mocked(updateApi.install).mockReset();
  vi.mocked(updateApi.state).mockResolvedValue({
    currentVersion: '0.1.0',
    phase: 'idle',
    availableVersion: null,
    announcementId: null,
    verifiedArtifactId: null,
    installationSupported: false,
  });
});

describe('unified SSH workspace', () => {
  let width = 1_000;
  let measure: ReturnType<typeof vi.spyOn>;
  const observers: Array<{ notify: () => void; disconnect: ReturnType<typeof vi.fn> }> = [];
  beforeEach(() => {
    width = 1_000;
    observers.length = 0;
    measure = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => ({
      width, height: 600, x: 0, y: 0, left: 0, top: 0, right: width, bottom: 600, toJSON() {},
    }));
    vi.stubGlobal('ResizeObserver', class {
      disconnect = vi.fn();
      constructor(notify: () => void) { observers.push({ notify, disconnect: this.disconnect }); }
      observe() {}
    });
    vi.stubGlobal('PointerEvent', class extends MouseEvent {
      pointerId: number;
      constructor(type: string, init: PointerEventInit) { super(type, init); this.pointerId = init.pointerId ?? 0; }
    });
    vi.mocked(hostApi.list).mockResolvedValue([host]);
    vi.mocked(hostApi.session).mockResolvedValue(connected);
    vi.mocked(terminalApi.open).mockImplementation(async (id, size) => ({ id: `${id}-pty`, hostId: id,
      hostSessionId: connected.hostSessionId!, label: 'Shell', size, state: 'open', error: null }));
    vi.mocked(filesApi.open).mockImplementation(async (id) => ({ id: `${id}-sftp`, hostId: id,
      hostSessionId: connected.hostSessionId!, protocolVersion: 3, rootPath: '/synthetic', extensions: [],
      limits: { maxPacketBytes: '262144', maxReadBytes: '65536', maxWriteBytes: '65536', maxOpenHandles: '64', clientChunkBytes: '65536', listingEntryCap: 5000 } }));
    vi.mocked(filesApi.list).mockImplementation(async (session, path) => ({ hostId: session.hostId,
      hostSessionId: session.hostSessionId, sftpSessionId: session.id, path, partial: false, entryCap: 5000,
      entries: [{ name: 'synthetic-file', displayName: 'synthetic-file', path: `${path}/synthetic-file`,
        kind: 'file', sizeBytes: '1', modifiedAt: null, permissions: null, uid: null, gid: null }] }));
  });
  afterEach(() => { measure.mockRestore(); vi.unstubAllGlobals(); });
  async function openTerminal() {
    const client = renderApp();
    await userEvent.click(await screen.findByRole('button', { name: `Open ${host.displayName}` }));
    await userEvent.click(screen.getByRole('button', { name: 'Terminal' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Open terminal' }));
    await screen.findByRole('textbox', { name: 'Synthetic Shell' });
    return client;
  }
  async function combine() {
    await userEvent.click(screen.getByRole('button', { name: 'Combine Terminal + Files' }));
    await screen.findByDisplayValue('/synthetic');
  }

  it('switches all three modes with stable PTY, SFTP, selection and private component state', async () => {
    const storage = vi.spyOn(Storage.prototype, 'setItem');
    try {
      const client = await openTerminal();
      const terminal = screen.getByTestId(`${host.id}-pty`);
      const text = screen.getByRole('textbox', { name: 'Synthetic Shell' });
      await userEvent.type(text, ' retained');
      await combine();
      const files = screen.getByRole('region', { name: `Files on ${host.displayName}` });
      const selected = screen.getByRole('checkbox', { name: 'Select synthetic-file' });
      await userEvent.click(selected);
      await userEvent.click(screen.getByRole('button', { name: 'Open full Files view' }));
      expect(terminal).not.toBeVisible();
      expect(selected).toBeChecked();
      await userEvent.click(screen.getByRole('button', { name: 'Terminal' }));
      expect(screen.getByTestId(`${host.id}-pty`)).toBe(terminal);
      expect(text).toHaveValue('Synthetic local scrollback retained');
      expect(screen.getByRole('region', { name: `Files on ${host.displayName}` })).toBe(files);
      await userEvent.click(screen.getByRole('button', { name: 'Hide files panel' }));
      expect(files).not.toBeVisible();
      await combine();
      expect(selected).toBeChecked();
      expect(terminalApi.open).toHaveBeenCalledOnce();
      expect(terminalApi.list).toHaveBeenCalledOnce();
      expect(filesApi.open).toHaveBeenCalledOnce();
      expect(filesApi.list).toHaveBeenCalledOnce();
      expect(JSON.stringify(client.getQueryCache().getAll().map((query) => query.state))).not.toContain('synthetic-file');
      expect(JSON.stringify(useSelection.getState())).not.toContain('synthetic-file');
      expect(storage).not.toHaveBeenCalled();
    } finally { storage.mockRestore(); }
  });

  it('clamps keyboard and captured pointer resizing to the container and releases drag on hide', async () => {
    await openTerminal();
    await combine();
    const separator = screen.getByRole('separator', { name: 'Resize files panel' });
    expect(separator).toHaveAttribute('aria-orientation', 'vertical');
    expect(Number(separator.getAttribute('aria-valuenow'))).toBeCloseTo(327, 0);
    fireEvent.keyDown(separator, { key: 'End' });
    expect(separator).toHaveAttribute('aria-valuenow', '592');
    fireEvent.keyDown(separator, { key: 'ArrowRight' });
    expect(separator).toHaveAttribute('aria-valuenow', '592');
    fireEvent.keyDown(separator, { key: 'Home' });
    fireEvent.keyDown(separator, { key: 'ArrowLeft' });
    expect(separator).toHaveAttribute('aria-valuenow', '280');
    fireEvent.keyDown(separator, { key: 'ArrowRight' });
    expect(separator).toHaveAttribute('aria-valuenow', '296');
    let captured = false;
    separator.setPointerCapture = vi.fn(() => { captured = true; });
    separator.hasPointerCapture = vi.fn(() => captured);
    separator.releasePointerCapture = vi.fn(() => { captured = false; });
    fireEvent.pointerDown(separator, { pointerId: 4, button: 0, clientX: 300 });
    expect(separator.setPointerCapture).toHaveBeenCalledWith(4);
    fireEvent.pointerMove(separator, { pointerId: 4, clientX: 5_000 });
    expect(separator).toHaveAttribute('aria-valuenow', '592');
    await userEvent.click(screen.getByRole('button', { name: 'Hide files panel' }));
    expect(separator.releasePointerCapture).toHaveBeenCalledWith(4);
    const prior = separator.getAttribute('aria-valuenow');
    fireEvent.pointerMove(separator, { pointerId: 4, clientX: 0 });
    expect(separator).toHaveAttribute('aria-valuenow', prior);
  });

  it('preserves a Files-first workspace when Terminal is first opened later', async () => {
    renderApp();
    await userEvent.click(await screen.findByRole('button', { name: `Open ${host.displayName}` }));
    await userEvent.click(screen.getByRole('button', { name: 'Files' }));
    await screen.findByDisplayValue('/synthetic');
    const files = screen.getByRole('region', { name: `Files on ${host.displayName}` });
    const selection = screen.getByRole('checkbox', { name: 'Select synthetic-file' });
    await userEvent.click(selection);
    expect(terminalApi.list).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Terminal' }));
    await screen.findByRole('button', { name: 'Open terminal' });
    await combine();
    expect(screen.getByRole('region', { name: `Files on ${host.displayName}` })).toBe(files);
    expect(selection).toBeChecked();
    expect(filesApi.open).toHaveBeenCalledOnce();
    expect(filesApi.list).toHaveBeenCalledOnce();
  });

  it('releases pointer capture on cancellation and unmount and disconnects the layout observer', async () => {
    const client = await openTerminal();
    await combine();
    const separator = screen.getByRole('separator');
    let captured = false;
    separator.setPointerCapture = vi.fn(() => { captured = true; });
    separator.hasPointerCapture = vi.fn(() => captured);
    separator.releasePointerCapture = vi.fn(() => { captured = false; });
    fireEvent.pointerDown(separator, { pointerId: 8, button: 0, clientX: 300 });
    fireEvent.pointerCancel(separator, { pointerId: 8 });
    expect(separator.releasePointerCapture).toHaveBeenCalledTimes(1);
    fireEvent.pointerDown(separator, { pointerId: 9, button: 0, clientX: 300 });
    await act(async () => { client.setQueryData(['hosts'], []); });
    await waitFor(() => expect(separator).not.toBeInTheDocument());
    expect(separator.releasePointerCapture).toHaveBeenCalledTimes(2);
    expect(observers.at(-1)!.disconnect).toHaveBeenCalledOnce();
  });

  it('collapses at narrow container widths and opens full Files without moving the terminal', async () => {
    await openTerminal();
    await combine();
    const terminal = screen.getByTestId(`${host.id}-pty`);
    const files = screen.getByRole('region', { name: `Files on ${host.displayName}` });
    act(() => { width = 640; observers.forEach((observer) => observer.notify()); });
    expect(files).not.toBeVisible();
    expect(screen.queryByRole('separator')).not.toBeInTheDocument();
    expect(screen.getByText(/Files panel collapsed/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Open full Files view' }));
    expect(files).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: 'Terminal' }));
    expect(screen.getByTestId(`${host.id}-pty`)).toBe(terminal);
    act(() => { width = 900; observers.forEach((observer) => observer.notify()); });
    expect(files).toBeVisible();
    expect(Number(screen.getByRole('separator').getAttribute('aria-valuenow'))).toBeLessThanOrEqual(492);
    expect(filesApi.open).toHaveBeenCalledOnce();
  });

  it('starts only one compact sampler and keeps inactive host workspaces isolated', async () => {
    const other = { ...host, id: 'other-host', displayName: 'Other host' };
    vi.mocked(hostApi.list).mockResolvedValue([host, other]);
    vi.mocked(hostApi.session).mockImplementation(async (id) => ({ ...connected, hostId: id }));
    await openTerminal();
    await act(async () => { await Promise.resolve(); });
    const before = vi.mocked(monitorApi.sample).mock.calls.length;
    await combine();
    await waitFor(() => expect(monitorApi.sample).toHaveBeenCalledTimes(before + 1));
    expect(screen.getAllByRole('region', { name: 'SSH workspace monitoring' })).toHaveLength(1);
    await userEvent.selectOptions(screen.getByLabelText('Select a host'), other.id);
    expect(screen.queryByRole('region', { name: 'SSH workspace monitoring' })).not.toBeInTheDocument();
    expect(screen.queryByRole('region', { name: `Files on ${host.displayName}` })).not.toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText('Select a host'), host.id);
    expect(screen.getAllByRole('region', { name: `Files on ${host.displayName}` })).toHaveLength(1);
    expect(screen.getAllByRole('region', { name: 'SSH workspace monitoring' })).toHaveLength(1);
    expect(filesApi.open).toHaveBeenCalledOnce();
    expect(terminalApi.open).toHaveBeenCalledOnce();
  });
});

describe('manual update availability check', () => {
  it('does not check on mount, disables while pending, and allows a second manual check', async () => {
    let finishCheck!: (snapshot: UpdateOperationSnapshot) => void;
    vi.mocked(updateApi.check)
      .mockReturnValueOnce(
        new Promise((resolve) => {
          finishCheck = resolve;
        }),
      )
      .mockResolvedValueOnce({
        currentVersion: '0.1.0',
        phase: 'upToDate',
        availableVersion: null,
        announcementId: null,
        verifiedArtifactId: null,
        installationSupported: false,
      });
    renderApp();
    await screen.findByText('Your next server starts here.');
    await waitFor(() => expect(updateApi.state).toHaveBeenCalledTimes(1));
    expect(updateApi.check).not.toHaveBeenCalled();
    expect(updateApi.download).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    expect(updateApi.check).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('button', { name: 'Checking…' })).toBeDisabled();
    finishCheck({
      currentVersion: '0.1.0',
      phase: 'upToDate',
      availableVersion: null,
      announcementId: null,
      verifiedArtifactId: null,
      installationSupported: false,
    });
    expect(await screen.findByText('NexusOps 0.1.0 is up to date.')).toBeInTheDocument();
    const checkButton = screen.getByRole('button', { name: 'Check for updates' });
    expect(checkButton).toBeEnabled();
    expect(updateApi.check).toHaveBeenCalledTimes(1);

    await userEvent.click(checkButton);
    await waitFor(() => expect(updateApi.check).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('NexusOps 0.1.0 is up to date.')).toBeInTheDocument();
  });

  it('calls an available update announced and offers only explicit download and verify', async () => {
    vi.mocked(updateApi.check).mockResolvedValue({
      currentVersion: '0.1.0',
      phase: 'updateAnnounced',
      availableVersion: '0.2.0',
      announcementId: 'announcement-1',
      verifiedArtifactId: null,
      installationSupported: false,
    });
    renderApp();
    await waitFor(() => expect(updateApi.state).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    expect(await screen.findByText('Update 0.2.0 is announced.')).toBeInTheDocument();
    expect(
      screen.getByText('Download and verification require a separate explicit action.'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/verified/i)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Download and verify' })).toBeEnabled();
    expect(
      screen.queryByRole('button', { name: /install|restart|open release|copy url/i }),
    ).not.toBeInTheDocument();
  });

  it('downloads the exact announcement once and says verified only after the backend result', async () => {
    let finishDownload!: (snapshot: UpdateOperationSnapshot) => void;
    vi.mocked(updateApi.check).mockResolvedValue({
      currentVersion: '0.1.0',
      phase: 'updateAnnounced',
      availableVersion: '0.2.0',
      announcementId: 'announcement-1',
      verifiedArtifactId: null,
      installationSupported: false,
    });
    vi.mocked(updateApi.download).mockReturnValue(
      new Promise((resolve) => {
        finishDownload = resolve;
      }),
    );
    renderApp();
    await waitFor(() => expect(updateApi.state).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Download and verify' }));
    expect(updateApi.download).toHaveBeenCalledExactlyOnceWith('announcement-1');
    expect(screen.getByText('Downloading and verifying…')).toBeInTheDocument();
    expect(screen.queryByText(/was downloaded and verified/)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Downloading…' })).toBeDisabled();

    finishDownload({
      currentVersion: '0.1.0',
      phase: 'verified',
      availableVersion: '0.2.0',
      announcementId: null,
      verifiedArtifactId: null,
      installationSupported: false,
    });
    expect(
      await screen.findByText(
        'Update 0.2.0 was downloaded and verified against the NexusOps updater key; not installed.',
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Download and verify' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /install|restart/i })).not.toBeInTheDocument();
  });

  it('requires a second explicit Windows install action and cancel preserves authority', async () => {
    vi.mocked(updateApi.state).mockResolvedValue({
      currentVersion: '0.1.0',
      phase: 'verified',
      availableVersion: '0.2.0',
      announcementId: null,
      verifiedArtifactId: 'verified-1',
      installationSupported: true,
    });
    vi.mocked(updateApi.install).mockReturnValue(new Promise(() => {}));
    renderApp();
    const install = await screen.findByRole('button', { name: 'Install update' });
    await userEvent.click(install);
    expect(updateApi.install).not.toHaveBeenCalled();
    expect(screen.getByRole('group', { name: 'Confirm update installation' })).toHaveTextContent(
      'NexusOps will not reopen automatically',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(updateApi.install).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Install update' })).toBeEnabled();

    await userEvent.click(screen.getByRole('button', { name: 'Install update' }));
    await userEvent.click(screen.getByRole('button', { name: 'Install and close NexusOps' }));
    expect(updateApi.install).toHaveBeenCalledExactlyOnceWith('verified-1');
    expect(screen.getByRole('button', { name: 'Preparing installer…' })).toBeDisabled();
  });

  it('keeps non-Windows verified state neutral without an install action', async () => {
    vi.mocked(updateApi.state).mockResolvedValue({
      currentVersion: '0.1.0',
      phase: 'verified',
      availableVersion: '0.2.0',
      announcementId: null,
      verifiedArtifactId: null,
      installationSupported: false,
    });
    renderApp();
    expect(
      await screen.findByText('In-app installation is not enabled on this platform.'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /install/i })).not.toBeInTheDocument();
  });

  it('surfaces a safe sealed-state failure without retrying installation', async () => {
    vi.mocked(updateApi.state).mockResolvedValue({
      currentVersion: '0.1.0',
      phase: 'verified',
      availableVersion: '0.2.0',
      announcementId: null,
      verifiedArtifactId: 'verified-2',
      installationSupported: true,
    });
    vi.mocked(updateApi.install).mockRejectedValue({
      code: 'updateInstall',
      message:
        'The verified installer could not be launched. Close and restart NexusOps before continuing.',
      hostKey: null,
    });
    renderApp();
    await userEvent.click(await screen.findByRole('button', { name: 'Install update' }));
    await userEvent.click(screen.getByRole('button', { name: 'Install and close NexusOps' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Close and restart NexusOps');
    expect(updateApi.install).toHaveBeenCalledExactlyOnceWith('verified-2');
    expect(screen.getByRole('button', { name: 'Check for updates' })).toBeDisabled();
  });

  it('clears stale download authority on failure and requires a fresh check', async () => {
    vi.mocked(updateApi.check).mockResolvedValue({
      currentVersion: '0.1.0',
      phase: 'updateAnnounced',
      availableVersion: '0.2.0',
      announcementId: 'stale-id',
      verifiedArtifactId: null,
      installationSupported: false,
    });
    vi.mocked(updateApi.download).mockRejectedValue({
      code: 'updateConflict',
      message: 'The update state changed. Check for updates again.',
      hostKey: null,
    });
    renderApp();
    await waitFor(() => expect(updateApi.state).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Download and verify' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The update state changed. Check for updates again.',
    );
    expect(screen.queryByText('Update 0.2.0 is announced.')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Download and verify' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Check for updates' })).toBeEnabled();
  });

  it('removes stale success when a new manual check fails and shows the safe error', async () => {
    vi.mocked(updateApi.check)
      .mockResolvedValueOnce({
        currentVersion: '0.1.0',
        phase: 'upToDate',
        availableVersion: null,
        announcementId: null,
        verifiedArtifactId: null,
        installationSupported: false,
      })
      .mockRejectedValueOnce({
        code: 'updateCheck',
        message: 'The update service could not be checked right now.',
        hostKey: null,
      });
    renderApp();
    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    expect(await screen.findByText('NexusOps 0.1.0 is up to date.')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The update service could not be checked right now.',
    );
    expect(screen.queryByText('NexusOps 0.1.0 is up to date.')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Check for updates' })).toBeEnabled();
    expect(updateApi.check).toHaveBeenCalledTimes(2);
  });
});

describe('workspace flows', () => {
  it('keeps container metadata out of query, mutation, selection and browser storage', async () => {
    const canary = 'synthetic container memory-only assertion';
    vi.mocked(hostApi.list).mockResolvedValue([host]);
    vi.mocked(hostApi.session).mockResolvedValue(connected);
    vi.mocked(containersApi.list).mockResolvedValue({
      hostId: host.id, hostSessionId: connected.hostSessionId!, observedAt: '2026-09-28T10:00:00Z', provider: 'dockerSystem',
      entries: [{ id: 'a'.repeat(64), name: canary, image: 'fixture/image:1', state: 'running', status: 'Up', ports: '', networks: 'bridge' }],
    });
    const localWrite = vi.spyOn(Storage.prototype, 'setItem');
    const client = renderApp();
    await userEvent.click(await screen.findByRole('button', { name: 'Open Test gateway' }));
    await userEvent.click(screen.getByRole('button', { name: 'Containers' }));
    expect(await screen.findByText(canary)).toBeInTheDocument();
    expect(JSON.stringify(client.getQueryCache().getAll().map((query) => query.state))).not.toContain(canary);
    expect(JSON.stringify(client.getMutationCache().getAll().map((mutation) => mutation.state))).not.toContain(canary);
    expect(JSON.stringify(useSelection.getState())).not.toContain(canary);
    expect(localWrite).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Security' }));
    expect(screen.queryByText(canary)).not.toBeInTheDocument();
    localWrite.mockRestore();
  });
  it('keeps journal content out of query, mutation, selection and browser storage', async () => {
    const canary = 'synthetic logs memory-only assertion';
    vi.mocked(hostApi.list).mockResolvedValue([host]);
    vi.mocked(hostApi.session).mockResolvedValue(connected);
    vi.mocked(logsApi.list).mockResolvedValue({ hostId: host.id, hostSessionId: connected.hostSessionId!, observedAt: '2026-09-27T10:00:00Z', entries: [
      { timestamp: '2026-09-27T10:00:00.000000Z', priority: 'info', unit: null, identifier: null, messageState: 'text', message: canary },
    ] });
    const localWrite = vi.spyOn(Storage.prototype, 'setItem');
    const client = renderApp();
    await userEvent.click(await screen.findByRole('button', { name: 'Open Test gateway' }));
    await userEvent.click(screen.getByRole('button', { name: 'Logs' }));
    expect(await screen.findByText(canary)).toBeInTheDocument();
    expect(JSON.stringify(client.getQueryCache().getAll().map((query) => query.state))).not.toContain(canary);
    expect(JSON.stringify(client.getMutationCache().getAll().map((mutation) => mutation.state))).not.toContain(canary);
    expect(JSON.stringify(useSelection.getState())).not.toContain(canary);
    expect(localWrite).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Security' }));
    expect(screen.queryByText(canary)).not.toBeInTheDocument();
    localWrite.mockRestore();
  });
  it('enables Services, Network, Security, Logs and Containers for the selected host', async () => {
    vi.mocked(hostApi.list).mockResolvedValue([host]);
    renderApp();
    await userEvent.click(await screen.findByRole('button', { name: 'Open Test gateway' }));
    const services = screen.getByRole('button', { name: 'Services' });
    expect(services).toBeEnabled();
    await userEvent.click(services);
    expect(await screen.findByText('Connect this host to inspect system services.')).toBeInTheDocument();
    const network = screen.getByRole('button', { name: 'Network' });
    expect(network).toBeEnabled();
    await userEvent.click(network);
    expect(await screen.findByText('Connect this host to inspect network interfaces.')).toBeInTheDocument();
    const security = screen.getByRole('button', { name: 'Security' });
    expect(security).toBeEnabled();
    await userEvent.click(security);
    expect(await screen.findByRole('heading', { name: 'Security' })).toBeInTheDocument();
    expect(await screen.findByText('Not pinned')).toBeInTheDocument();
    const logs = screen.getByRole('button', { name: 'Logs' });
    expect(logs).toBeEnabled();
    await userEvent.click(logs);
    expect(await screen.findByText('Connect this host to inspect recent system journal entries.')).toBeInTheDocument();
    expect(logsApi.list).not.toHaveBeenCalled();
    const containers = screen.getByRole('button', { name: 'Containers' });
    expect(containers).toBeEnabled();
    await userEvent.click(containers);
    expect(await screen.findByText('Connect this host to inspect local Docker containers.')).toBeInTheDocument();
    expect(containersApi.list).not.toHaveBeenCalled();
  });
  it('has an honest empty state and disables host workspaces without a selection', async () => {
    renderApp();
    expect(await screen.findByText('Your next server starts here.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Containers' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Services' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Logs' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Security' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Network' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Terminal' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Files' })).toBeDisabled();
  });
  it('saves credentials without retaining them in query caches, UI store or browser storage', async () => {
    const localWrite = vi.spyOn(Storage.prototype, 'setItem');
    let hosts: Host[] = [];
    vi.mocked(hostApi.list).mockImplementation(async () => hosts);
    vi.mocked(hostApi.save).mockImplementation(async (input) => {
      const saved = { ...input, id: host.id };
      hosts = [saved];
      return saved;
    });
    const client = renderApp();
    await userEvent.click(await screen.findByRole('button', { name: 'Add your first host' }));
    await userEvent.type(screen.getByLabelText('Display name'), 'Test gateway');
    await userEvent.type(screen.getByLabelText('Hostname or IP'), 'gateway.example.test');
    await userEvent.type(screen.getByLabelText('Username'), 'deploy');
    await userEvent.selectOptions(screen.getByLabelText('Method'), 'password');
    await userEvent.type(screen.getByLabelText('Password'), 'never-cache-this-secret');
    await userEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Add host' }),
    );
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(hostApi.save).toHaveBeenCalledWith(
      expect.objectContaining({ displayName: 'Test gateway' }),
      { password: 'never-cache-this-secret', privateKey: null, passphrase: null },
    );
    expect(
      JSON.stringify(
        client
          .getQueryCache()
          .getAll()
          .map((query) => query.state),
      ),
    ).not.toContain('never-cache-this-secret');
    expect(
      JSON.stringify(
        client
          .getMutationCache()
          .getAll()
          .map((mutation) => mutation.state),
      ),
    ).not.toContain('never-cache-this-secret');
    expect(JSON.stringify(useSelection.getState())).not.toContain('never-cache-this-secret');
    expect(localWrite).not.toHaveBeenCalled();
  });
  it('never displays another host’s discovery after switching selection', async () => {
    const other = {
      ...host,
      id: 'test-host-2',
      displayName: 'Test worker',
      connection: { ...host.connection, hostname: 'worker.example.test' },
    };
    vi.mocked(hostApi.list).mockResolvedValue([host, other]);
    vi.mocked(hostApi.session).mockImplementation(async (id) =>
      id === host.id ? connected : { ...disconnected, hostId: id },
    );
    renderApp();
    await screen.findByRole('button', { name: `Open ${host.displayName}` });
    await userEvent.selectOptions(screen.getByLabelText('Select a host'), host.id);
    expect(await screen.findByText('Debian GNU/Linux')).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText('Select a host'), other.id);
    expect(await screen.findByRole('heading', { name: 'Test worker' })).toBeInTheDocument();
    expect(screen.queryByText('Debian GNU/Linux')).not.toBeInTheDocument();
    expect(screen.getByText('Ready when you are')).toBeInTheDocument();
  });
});

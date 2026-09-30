import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Host, UpdateOperationSnapshot } from '@nexusops/protocol';
import App from './App';
import { containersApi, hostApi, securityApi, logsApi, updateApi } from './api/client';
import { useSelection } from './state/selection';
import { connected, disconnected, host } from './test/fixtures';

vi.mock('./api/client', async (original) => ({
  ...(await original<typeof import('./api/client')>()),
  securityApi: { get: vi.fn() },
  logsApi: { list: vi.fn() },
  containersApi: { list: vi.fn() },
  updateApi: { state: vi.fn(), check: vi.fn(), download: vi.fn() },
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
  vi.mocked(updateApi.state).mockResolvedValue({
    currentVersion: '0.1.0',
    phase: 'idle',
    availableVersion: null,
    announcementId: null,
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
    });
    renderApp();
    await waitFor(() => expect(updateApi.state).toHaveBeenCalledTimes(1));
    await userEvent.click(screen.getByRole('button', { name: 'Check for updates' }));
    expect(await screen.findByText('Update 0.2.0 is announced.')).toBeInTheDocument();
    expect(
      screen.getByText('Download is explicit. Installation is not enabled in this build.'),
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
    });
    expect(
      await screen.findByText(
        'Update 0.2.0 was downloaded and verified against the NexusOps updater key.',
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Download and verify' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /install|restart/i })).not.toBeInTheDocument();
  });

  it('clears stale download authority on failure and requires a fresh check', async () => {
    vi.mocked(updateApi.check).mockResolvedValue({
      currentVersion: '0.1.0',
      phase: 'updateAnnounced',
      availableVersion: '0.2.0',
      announcementId: 'stale-id',
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

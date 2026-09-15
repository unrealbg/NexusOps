import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Host } from '@nexusops/protocol';
import App from './App';
import { hostApi } from './api/client';
import { useSelection } from './state/selection';
import { connected, disconnected, host } from './test/fixtures';

vi.mock('./api/client', async (original) => ({
  ...(await original<typeof import('./api/client')>()),
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
});

describe('workspace flows', () => {
  it('has an honest empty state and marks future navigation unavailable', async () => {
    renderApp();
    expect(await screen.findByText('Your next server starts here.')).toBeInTheDocument();
    for (const label of [
      'Services',
      'Containers',
      'Network',
      'Security',
      'Logs',
    ])
      expect(screen.getByRole('button', { name: `${label} (coming soon)` })).toBeDisabled();
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

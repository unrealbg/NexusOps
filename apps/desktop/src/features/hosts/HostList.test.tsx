import { QueryClient, QueryClientProvider, focusManager } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ConnectionState, Host, HostSession } from '@nexusops/protocol';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { hostApi } from '../../api/client';
import { hostKeys } from '../../api/queries';
import { HostSidebar } from '../../components/HostSidebar';
import { HostList } from './HostList';

vi.mock('../../api/client', () => ({ hostApi: { session: vi.fn() } }));

function host(id: string, displayName: string, hostname: string, username: string): Host {
  return {
    id,
    displayName,
    connection: { hostname, username, port: 22, authentication: 'privateKey' },
  };
}

const savedHosts: [Host, Host, Host] = [
  host('host-a', 'Alpha', 'zulu.example.test', 'ops-admin'),
  host('host-b', 'Beta', 'alpha.example.test', 'deploy'),
  host('host-c', 'Gamma', 'middle.example.test', 'reader'),
];

function session(hostId: string, state: ConnectionState = 'disconnected'): HostSession {
  return {
    hostId,
    hostSessionId: state === 'connected' ? `session-${hostId}` : null,
    state,
    error: null,
    identity: null,
    capabilities: [],
    discovery: null,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

const clients: QueryClient[] = [];

function renderList(hosts: Host[] = savedHosts, withSidebar = false) {
  const client = new QueryClient({
    // The component must override retry rather than inheriting this test default.
    defaultOptions: { queries: { retry: 2, gcTime: Infinity, refetchOnWindowFocus: false } },
  });
  clients.push(client);
  const onSelect = vi.fn();
  const onAdd = vi.fn();
  const tree = (currentHosts: Host[]) => (
    <QueryClientProvider client={client}>
      <HostList hosts={currentHosts} onSelect={onSelect} onAdd={onAdd} />
      {withSidebar && (
        <HostSidebar
          hosts={currentHosts}
          selectedId={null}
          onSelect={vi.fn()}
          onAdd={vi.fn()}
          activeSection="overview"
          onSection={vi.fn()}
        />
      )}
    </QueryClientProvider>
  );
  const view = render(tree(hosts));
  return {
    ...view,
    client,
    onSelect,
    onAdd,
    rerenderHosts: (next: Host[]) => view.rerender(tree(next)),
  };
}

function names() {
  return screen
    .queryAllByRole('button', { name: /^Open / })
    .map((button) => button.getAttribute('aria-label')!.slice('Open '.length));
}

function hostRow(name: string) {
  return screen.getByRole('button', { name: `Open ${name}` }).closest('tr')!;
}

function usernames() {
  return within(screen.getByRole('table'))
    .getAllByRole('row')
    .slice(1)
    .map((row) => within(row).getAllByRole('cell')[1]?.textContent);
}

async function tick(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

beforeEach(() => {
  vi.mocked(hostApi.session)
    .mockReset()
    .mockImplementation(async (id) => session(id));
});

afterEach(() => {
  cleanup();
  clients.forEach((client) => client.clear());
  clients.length = 0;
  vi.useRealTimers();
  focusManager.setFocused(undefined);
});

describe('local host search and sorting', () => {
  it.each([
    ['Gamma', ['Gamma']],
    ['Alpha', ['Alpha', 'Beta']],
    ['alpha.example.test', ['Beta']],
    ['deploy', ['Beta']],
    ['  oPS-Ad  ', ['Alpha']],
    ['MiDdLe.ExAm', ['Gamma']],
    ['  bEt  ', ['Beta']],
  ])('searches name, hostname or username using %j', async (term, expected) => {
    renderList();
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search hosts' }), term);
    expect(names()).toEqual(expected);
    expect(screen.getByRole('status')).toHaveTextContent(`Showing ${expected.length} of 3 hosts`);
  });

  it.each([
    ['name-asc', ['Alpha', 'Beta', 'Gamma']],
    ['name-desc', ['Gamma', 'Beta', 'Alpha']],
    ['hostname-asc', ['Beta', 'Gamma', 'Alpha']],
    ['hostname-desc', ['Alpha', 'Gamma', 'Beta']],
  ])('sorts by %s without mutating saved hosts', async (sort, expected) => {
    const hosts = [...savedHosts].reverse();
    const original = hosts.map((entry) => entry.id);
    Object.freeze(hosts);
    renderList(hosts);
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Sort hosts' }), sort);
    expect(names()).toEqual(expected);
    expect(hosts.map((entry) => entry.id)).toEqual(original);
  });

  it.each(['name-asc', 'name-desc', 'hostname-asc', 'hostname-desc'])(
    'breaks case-insensitive %s ties by ascending immutable ID',
    async (sort) => {
      const hosts = [
        host('id-z', 'Same', 'SAME.example.test', 'last-id'),
        host('id-a', 'same', 'same.example.test', 'first-id'),
        host('id-m', 'SAME', 'Same.example.test', 'middle-id'),
      ];
      renderList(hosts);
      await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Sort hosts' }), sort);
      expect(usernames()).toEqual(['first-id', 'middle-id', 'last-id']);
    },
  );

  it('clears search, status and sort and restores the full list and keyboard focus', async () => {
    const view = renderList();
    const user = userEvent.setup();
    await user.type(screen.getByRole('searchbox', { name: 'Search hosts' }), 'Alpha');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Sort hosts' }), 'name-desc');
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Connection status' }),
      'connected',
    );
    expect(screen.getByRole('heading', { name: 'No matching hosts' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(names()).toEqual(['Alpha', 'Beta', 'Gamma']);
    expect(screen.getByRole('searchbox', { name: 'Search hosts' })).toHaveValue('');
    expect(screen.getByRole('searchbox', { name: 'Search hosts' })).toHaveFocus();
    expect(screen.getByRole('combobox', { name: 'Sort hosts' })).toHaveValue('name-asc');
    expect(screen.getByRole('combobox', { name: 'Connection status' })).toHaveValue('all');
    expect(screen.queryByRole('button', { name: 'Clear filters' })).not.toBeInTheDocument();
    expect(view.onSelect).not.toHaveBeenCalled();
  });

  it('distinguishes no saved hosts from no matches and preserves both Add Host actions', async () => {
    const view = renderList([]);
    expect(
      screen.getByRole('heading', { name: 'Your next server starts here.' }),
    ).toBeInTheDocument();
    expect(screen.getByText(/No saved hosts yet\./)).toBeInTheDocument();
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    expect(hostApi.session).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Add host' }));
    await userEvent.click(screen.getByRole('button', { name: 'Add your first host' }));
    expect(view.onAdd).toHaveBeenCalledTimes(2);
    view.rerenderHosts(savedHosts);
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search hosts' }), 'no-such-host');
    expect(screen.getByRole('heading', { name: 'No matching hosts' })).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Showing 0 of 3 hosts');
    expect(screen.queryByText(/No saved hosts yet\./)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Add host' })).toBeEnabled();
  });

  it('selects the correct immutable ID after sorting and filtering, without automatic selection', async () => {
    const view = renderList();
    const user = userEvent.setup();
    await user.selectOptions(screen.getByRole('combobox', { name: 'Sort hosts' }), 'hostname-desc');
    await user.type(screen.getByRole('searchbox', { name: 'Search hosts' }), 'alpha.example');
    expect(names()).toEqual(['Beta']);
    expect(view.onSelect).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Open Beta' }));
    expect(view.onSelect).toHaveBeenLastCalledWith('host-b');
    await user.click(
      within(hostRow('Beta')).getByRole('button', { name: /Beta.*alpha.example.test/ }),
    );
    expect(view.onSelect).toHaveBeenCalledTimes(2);
    expect(view.onSelect).toHaveBeenLastCalledWith('host-b');
  });

  it('provides named controls, live counts and keyboard-operable search, clear and host actions', async () => {
    const view = renderList();
    const user = userEvent.setup();
    await user.tab();
    expect(screen.getByRole('button', { name: 'Add host' })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('searchbox', { name: 'Search hosts' })).toHaveFocus();
    await user.keyboard('Beta');
    expect(names()).toEqual(['Beta']);
    await user.tab();
    expect(screen.getByRole('combobox', { name: 'Sort hosts' })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('combobox', { name: 'Connection status' })).toHaveFocus();
    expect(screen.getByRole('combobox', { name: 'Connection status' })).toHaveAccessibleDescription(
      /Other known states include connecting/,
    );
    await user.tab();
    expect(screen.getByRole('button', { name: 'Clear filters' })).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(screen.getByRole('searchbox', { name: 'Search hosts' })).toHaveFocus();
    expect(screen.getByRole('status')).toHaveAttribute('aria-live', 'polite');
    expect(names()).toHaveLength(3);
    within(hostRow('Alpha'))
      .getByRole('button', { name: /Alpha.*zulu.example.test/ })
      .focus();
    await user.keyboard('{Enter}');
    expect(view.onSelect).toHaveBeenCalledWith('host-a');
  });
});

describe('backend session observations and local status filters', () => {
  const knownStates: ConnectionState[] = [
    'connected',
    'disconnected',
    'connecting',
    'awaitingTrust',
    'disconnecting',
    'failed',
  ];
  const statusHosts = [
    ...knownStates.map((state) => host(state, state, `${state}.example.test`, 'synthetic-user')),
    host('pending', 'Pending', 'pending.example.test', 'synthetic-user'),
    host('unavailable', 'Unavailable', 'unavailable.example.test', 'synthetic-user'),
  ];

  it.each([
    ['connected', ['connected']],
    ['disconnected', ['disconnected']],
    ['other', ['awaitingTrust', 'connecting', 'disconnecting', 'failed']],
    ['unknown', ['Pending', 'Unavailable']],
  ])(
    'filters %s without confusing observation errors and connection failures',
    async (filter, expected) => {
      const pending = deferred<HostSession>();
      vi.mocked(hostApi.session).mockImplementation((id) =>
        id === 'pending'
          ? pending.promise
          : id === 'unavailable'
            ? Promise.reject(new Error('Synthetic observation unavailable'))
            : Promise.resolve(session(id, id as ConnectionState)),
      );
      renderList(statusHosts);
      await screen.findByText('Connection failed');
      await userEvent.selectOptions(
        screen.getByRole('combobox', { name: 'Connection status' }),
        filter,
      );
      expect(names()).toEqual(expected);
      if (filter === 'unknown') {
        expect(within(hostRow('Pending')).getByText('Loading…')).toBeInTheDocument();
        expect(within(hostRow('Unavailable')).getByText('Status unavailable')).toBeInTheDocument();
        expect(within(hostRow('Unavailable')).queryByText('Disconnected')).not.toBeInTheDocument();
      }
    },
  );

  it('rejects wrong-host observations instead of giving them a connected badge', async () => {
    vi.mocked(hostApi.session).mockResolvedValue(session('another-host', 'connected'));
    renderList([savedHosts[0]]);
    await screen.findByText('Status unavailable');
    await userEvent.selectOptions(
      screen.getByRole('combobox', { name: 'Connection status' }),
      'connected',
    );
    expect(names()).toEqual([]);
    await userEvent.selectOptions(
      screen.getByRole('combobox', { name: 'Connection status' }),
      'unknown',
    );
    expect(names()).toEqual(['Alpha']);
  });

  it('treats a failed refresh as unknown even when the cache retains a connected snapshot', async () => {
    vi.mocked(hostApi.session).mockResolvedValue(session('host-a', 'connected'));
    const view = renderList([savedHosts[0]]);
    await within(hostRow('Alpha')).findByText('Connected');
    await userEvent.selectOptions(
      screen.getByRole('combobox', { name: 'Connection status' }),
      'connected',
    );
    vi.mocked(hostApi.session).mockRejectedValue(new Error('Synthetic observation unavailable'));
    await act(async () => {
      await view.client.refetchQueries({ queryKey: hostKeys.session('host-a'), exact: true });
    });
    await screen.findByRole('heading', { name: 'No matching hosts' });
    expect(view.client.getQueryData<HostSession>(hostKeys.session('host-a'))?.state).toBe(
      'connected',
    );
    await userEvent.selectOptions(
      screen.getByRole('combobox', { name: 'Connection status' }),
      'unknown',
    );
    expect(within(hostRow('Alpha')).getByText('Status unavailable')).toBeInTheDocument();
    expect(hostApi.session).toHaveBeenCalledTimes(2);
  });

  it('makes a hidden host reappear from new session data while a filter stays active', async () => {
    const view = renderList(savedHosts.slice(0, 2));
    await within(hostRow('Alpha')).findByText('Disconnected');
    await userEvent.selectOptions(
      screen.getByRole('combobox', { name: 'Connection status' }),
      'connected',
    );
    expect(names()).toEqual([]);
    for (const entry of savedHosts.slice(0, 2)) {
      expect(
        view.client
          .getQueryCache()
          .find({ queryKey: hostKeys.session(entry.id) })
          ?.getObserversCount(),
      ).toBe(1);
    }
    await act(async () => {
      view.client.setQueryData(hostKeys.session('host-b'), session('host-b', 'connected'));
    });
    await screen.findByRole('button', { name: 'Open Beta' });
    expect(screen.getByRole('combobox', { name: 'Connection status' })).toHaveValue('connected');
    await act(async () => {
      view.client.setQueryData(hostKeys.session('host-b'), session('host-b'));
    });
    await screen.findByRole('heading', { name: 'No matching hosts' });
    expect(hostApi.session).toHaveBeenCalledTimes(2);
  });

  it('keeps observations bound to IDs when saved hosts are reordered, added or removed', async () => {
    vi.mocked(hostApi.session).mockImplementation(async (id) =>
      session(id, id === 'host-a' ? 'connected' : 'disconnected'),
    );
    const view = renderList([savedHosts[0]]);
    await within(hostRow('Alpha')).findByText('Connected');
    view.rerenderHosts([savedHosts[1], savedHosts[0]]);
    await within(hostRow('Beta')).findByText('Disconnected');
    expect(within(hostRow('Alpha')).getByText('Connected')).toBeInTheDocument();
    expect(hostApi.session).toHaveBeenCalledTimes(2);
    view.rerenderHosts([savedHosts[1]]);
    expect(names()).toEqual(['Beta']);
    expect(
      view.client
        .getQueryCache()
        .find({ queryKey: hostKeys.session('host-a') })
        ?.getObserversCount(),
    ).toBe(0);
    expect(
      view.client
        .getQueryCache()
        .find({ queryKey: hostKeys.session('host-b') })
        ?.getObserversCount(),
    ).toBe(1);
  });
});

describe('stable session polling ownership', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
    focusManager.setFocused(true);
  });

  it('polls hidden hosts once per three-second interval, not on local view changes or after unmount', async () => {
    let betaConnected = false;
    vi.mocked(hostApi.session).mockImplementation(async (id) =>
      session(id, id === 'host-b' && betaConnected ? 'connected' : 'disconnected'),
    );
    const view = renderList(savedHosts.slice(0, 2));
    await tick(1);
    expect(hostApi.session).toHaveBeenCalledTimes(2);
    fireEvent.change(screen.getByRole('combobox', { name: 'Connection status' }), {
      target: { value: 'connected' },
    });
    expect(names()).toEqual([]);
    fireEvent.change(screen.getByRole('combobox', { name: 'Sort hosts' }), {
      target: { value: 'hostname-desc' },
    });
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search hosts' }), {
      target: { value: 'example.test' },
    });
    expect(hostApi.session).toHaveBeenCalledTimes(2);
    await tick(2_998);
    expect(hostApi.session).toHaveBeenCalledTimes(2);
    betaConnected = true;
    await tick(2);
    expect(hostApi.session).toHaveBeenCalledTimes(4);
    expect(names()).toEqual(['Beta']);
    for (const entry of savedHosts.slice(0, 2)) {
      expect(
        view.client
          .getQueryCache()
          .find({ queryKey: hostKeys.session(entry.id) })
          ?.getObserversCount(),
      ).toBe(1);
    }
    view.unmount();
    await tick(6_000);
    expect(hostApi.session).toHaveBeenCalledTimes(4);
  });

  it('shares cache identities and deduplicates in-flight polling with the unchanged HostSidebar', async () => {
    const first = deferred<HostSession>();
    const next = deferred<HostSession>();
    vi.mocked(hostApi.session).mockReturnValueOnce(first.promise).mockReturnValue(next.promise);
    const view = renderList([savedHosts[0]], true);
    await tick(0);
    expect(hostApi.session).toHaveBeenCalledTimes(1);
    expect(view.client.getQueryCache().getAll()).toHaveLength(1);
    expect(
      view.client
        .getQueryCache()
        .find({ queryKey: hostKeys.session('host-a') })
        ?.getObserversCount(),
    ).toBe(2);
    await act(async () => {
      first.resolve(session('host-a', 'connected'));
    });
    await tick(1);
    fireEvent.change(screen.getByRole('combobox', { name: 'Connection status' }), {
      target: { value: 'unknown' },
    });
    expect(names()).toEqual([]);
    await tick(2_998);
    expect(hostApi.session).toHaveBeenCalledTimes(1);
    await tick(2);
    expect(hostApi.session).toHaveBeenCalledTimes(2);
    await tick(6_000);
    expect(hostApi.session).toHaveBeenCalledTimes(2);
    expect(
      view.client
        .getQueryCache()
        .find({ queryKey: hostKeys.session('host-a') })
        ?.getObserversCount(),
    ).toBe(2);
  });

  it('does not retry failed observations between the existing polling ticks', async () => {
    vi.mocked(hostApi.session).mockRejectedValue(new Error('Synthetic observation unavailable'));
    renderList([savedHosts[0]]);
    await tick(1);
    expect(within(hostRow('Alpha')).getByText('Status unavailable')).toBeInTheDocument();
    expect(hostApi.session).toHaveBeenCalledTimes(1);
    await tick(2_998);
    expect(hostApi.session).toHaveBeenCalledTimes(1);
    await tick(2);
    expect(hostApi.session).toHaveBeenCalledTimes(2);
    expect(within(hostRow('Alpha')).queryByText('Disconnected')).not.toBeInTheDocument();
  });
});

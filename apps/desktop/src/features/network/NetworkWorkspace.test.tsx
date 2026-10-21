import { StrictMode } from 'react';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type {
  DnsConfigurationSnapshot,
  Host,
  ListeningPortSnapshot,
  NetworkRouteSnapshot,
  NetworkSnapshot,
} from '@nexusops/protocol';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  state: 'disconnected',
  sessionId: null as string | null,
  list: vi.fn(),
  routes: vi.fn(),
  listeningPorts: vi.fn(),
  dnsConfiguration: vi.fn(),
}));
vi.mock('../../api/queries', () => ({
  useHostSession: () => ({ data: { state: mocks.state, hostSessionId: mocks.sessionId } }),
}));
vi.mock('../../api/client', () => ({
  networkApi: {
    list: mocks.list,
    routes: mocks.routes,
    listeningPorts: mocks.listeningPorts,
    dnsConfiguration: mocks.dnsConfiguration,
  },
}));

import { NetworkWorkspace } from './NetworkWorkspace';

const A = '11111111-1111-4111-8111-111111111111';
const B = '22222222-2222-4222-8222-222222222222';
const host = { id: 'host-a', displayName: 'Alpha' } as Host;
const otherHost = { id: 'host-b', displayName: 'Beta' } as Host;
function snapshot(hostId = host.id, hostSessionId = A): NetworkSnapshot {
  return {
    hostId,
    hostSessionId,
    observedAt: '2026-09-26T12:00:00Z',
    entries: [
      {
        ifindex: 1,
        name: 'lo',
        operState: null,
        mtu: null,
        addresses: [{ family: 'ipv4', address: '127.0.0.1', prefixLength: 8 }],
      },
      {
        ifindex: 2,
        name: '<img src=x>',
        operState: 'FUTURE_STATE',
        mtu: 1500,
        addresses: [{ family: 'ipv6', address: '2001:db8::10', prefixLength: 64 }],
      },
    ],
  };
}
function freshSnapshot(hostSessionId: string): NetworkSnapshot {
  const result = snapshot(host.id, hostSessionId);
  result.entries[0] = {
    ifindex: 1,
    name: 'lo',
    operState: null,
    mtu: null,
    addresses: [{ family: 'ipv4', address: '127.0.0.2', prefixLength: 8 }],
  };
  return result;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
beforeEach(() => {
  mocks.state = 'disconnected';
  mocks.sessionId = null;
  mocks.list.mockReset();
  mocks.listeningPorts.mockReset().mockResolvedValue({
    hostId: host.id,
    hostSessionId: A,
    observedAt: '2026-10-21T12:00:00Z',
    entries: [],
  });
  mocks.dnsConfiguration.mockReset().mockResolvedValue({
    hostId: host.id,
    hostSessionId: A,
    observedAt: '2026-10-21T12:01:00Z',
    nameservers: [],
    searchDirective: null,
    searchDomains: [],
    options: [],
    sortlist: [],
  });
  mocks.routes.mockReset().mockResolvedValue({
    hostId: host.id,
    hostSessionId: A,
    observedAt: '2026-10-21T00:00:00Z',
    entries: [],
  });
});

describe('session-bound Network workspace', () => {
  it('makes no request for a disconnected host', () => {
    render(<NetworkWorkspace host={host} />);
    expect(
      screen.getByText('Connect this host to inspect network interfaces.'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    expect(mocks.list).not.toHaveBeenCalled();
    expect(mocks.dnsConfiguration).not.toHaveBeenCalled();
  });

  it('loads once, renders validated text and optional fields, and filters without remote calls', async () => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    mocks.list.mockResolvedValue(snapshot());
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('<img src=x>')).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    expect(screen.getByText('IPv6 2001:db8::10/64')).toBeInTheDocument();
    expect(screen.getByText('2 interfaces · 2 addresses', { exact: false })).toBeInTheDocument();
    expect(screen.getAllByText('—')).toHaveLength(2);
    expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A);
    await userEvent.type(
      screen.getByRole('searchbox', { name: 'Filter interfaces and addresses' }),
      '2001:db8',
    );
    expect(screen.queryByText('lo')).not.toBeInTheDocument();
    expect(screen.getByText('<img src=x>')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledTimes(1);
    await userEvent.clear(
      screen.getByRole('searchbox', { name: 'Filter interfaces and addresses' }),
    );
    await userEvent.type(
      screen.getByRole('searchbox', { name: 'Filter interfaces and addresses' }),
      'no-match',
    );
    expect(screen.getByText('No interfaces match this filter.')).toBeInTheDocument();
  });

  it('refreshes once while pending and keeps only a same-session snapshot on failure', async () => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    const refresh = deferred<NetworkSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(refresh.promise);
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    const button = screen.getByRole('button', { name: 'Refresh' });
    await userEvent.click(button);
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(mocks.list).toHaveBeenCalledTimes(2);
    await act(async () => {
      refresh.reject(new Error('unavailable'));
    });
    expect(
      await screen.findByText('Refresh failed — showing the last successful snapshot.'),
    ).toBeInTheDocument();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    await waitFor(() => expect(button).toBeEnabled());
  });

  it('distinguishes an empty success from initial unavailability', async () => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({ ...snapshot(), entries: [] });
    const view = render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('No network interfaces were returned.')).toBeInTheDocument();
    view.unmount();
    mocks.list.mockRejectedValueOnce(new Error('unsupported'));
    render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText('Network inventory is unavailable on this host.'),
    ).toBeInTheDocument();
  });

  it('hides disconnected data, requests a new session, and drops delayed old-session data', async () => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    const old = deferred<NetworkSnapshot>();
    const fresh = freshSnapshot(B);
    mocks.list.mockReturnValueOnce(old.promise).mockResolvedValueOnce(fresh);
    const view = render(<NetworkWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(1));
    mocks.state = 'disconnected';
    mocks.sessionId = null;
    view.rerender(<NetworkWorkspace host={host} />);
    expect(
      screen.getByText('Connect this host to inspect network interfaces.'),
    ).toBeInTheDocument();
    mocks.state = 'connected';
    mocks.sessionId = B;
    view.rerender(<NetworkWorkspace host={host} />);
    await act(async () => {
      old.resolve(snapshot());
    });
    expect(await screen.findByText('IPv4 127.0.0.2/8')).toBeInTheDocument();
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith(host.id, B);
    expect(mocks.list).toHaveBeenCalledTimes(2);
  });

  it('removes a visible snapshot on disconnect and does not reuse it on reconnect', async () => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    const next = deferred<NetworkSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(next.promise);
    const view = render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    mocks.state = 'disconnected';
    mocks.sessionId = null;
    view.rerender(<NetworkWorkspace host={host} />);
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    mocks.state = 'connected';
    mocks.sessionId = B;
    view.rerender(<NetworkWorkspace host={host} />);
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    await act(async () => {
      next.resolve(freshSnapshot(B));
    });
    expect(screen.getByText('IPv4 127.0.0.2/8')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledTimes(2);
  });

  it('drops old-host and unmounted responses', async () => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    const oldHost = deferred<NetworkSnapshot>();
    const unmounted = deferred<NetworkSnapshot>();
    mocks.list.mockReturnValueOnce(oldHost.promise).mockReturnValueOnce(unmounted.promise);
    const view = render(<NetworkWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(1));
    view.rerender(<NetworkWorkspace host={otherHost} />);
    await act(async () => {
      oldHost.resolve(snapshot());
    });
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith(otherHost.id, A);
    view.unmount();
    await act(async () => {
      unmounted.resolve(snapshot(otherHost.id));
    });
  });

  it('rejects mismatched host and session identities from the backend', async () => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce(snapshot(otherHost.id, A));
    const view = render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText('Network inventory is unavailable on this host.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    view.unmount();
    mocks.list.mockResolvedValueOnce(snapshot(host.id, B));
    render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText('Network inventory is unavailable on this host.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
  });
});

function routeSnapshot(hostId = host.id, hostSessionId = A): NetworkRouteSnapshot {
  return {
    hostId,
    hostSessionId,
    observedAt: '2026-10-21T00:00:00Z',
    entries: [
      {
        family: 'ipv4',
        destination: 'default',
        gateway: '192.0.2.1',
        interface: 'eth0',
        metric: null,
        onLink: false,
        linkDown: false,
        preference: null,
      },
      {
        family: 'ipv4',
        destination: 'default',
        gateway: '198.51.100.1',
        interface: 'eth1',
        metric: 200,
        onLink: true,
        linkDown: false,
        preference: null,
      },
      {
        family: 'ipv6',
        destination: '2001:db8::/64',
        gateway: null,
        interface: 'eth0',
        metric: 0,
        onLink: false,
        linkDown: true,
        preference: 'medium',
      },
    ],
  };
}
function routeSection() {
  return within(screen.getByRole('region', { name: 'Routes & Gateway' }));
}

describe('Routes & Gateway', () => {
  beforeEach(() => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    mocks.list.mockResolvedValue(snapshot());
    mocks.routes.mockResolvedValue(routeSnapshot());
  });

  it('serializes initial and manual observations and prevents overlapping Refresh', async () => {
    const addresses = deferred<NetworkSnapshot>();
    const routes = deferred<NetworkRouteSnapshot>();
    mocks.list.mockReturnValueOnce(addresses.promise);
    mocks.routes.mockReturnValueOnce(routes.promise);
    render(<NetworkWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A));
    expect(mocks.routes).not.toHaveBeenCalled();
    await act(async () => {
      addresses.resolve(snapshot());
    });
    expect(mocks.routes).toHaveBeenCalledExactlyOnceWith(host.id, A);
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    await act(async () => {
      routes.resolve(routeSnapshot());
    });
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled();
    const refreshAddresses = deferred<NetworkSnapshot>();
    mocks.list.mockReturnValueOnce(refreshAddresses.promise);
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(mocks.list).toHaveBeenCalledTimes(2);
    expect(mocks.routes).toHaveBeenCalledTimes(1);
    await act(async () => {
      refreshAddresses.resolve(snapshot());
    });
    await waitFor(() => expect(mocks.routes).toHaveBeenCalledTimes(2));
  });

  it('renders actual multiple defaults, absent versus zero metrics, direct routes, flags and preference', async () => {
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('192.0.2.1')).toBeInTheDocument();
    const section = routeSection();
    expect(section.getByText('198.51.100.1 (on-link)')).toBeInTheDocument();
    expect(section.getAllByText('default')).toHaveLength(2);
    expect(
      section.getByText('2 default route(s) observed; no uniquely preferred gateway is inferred.'),
    ).toBeInTheDocument();
    expect(section.getByText('Direct / on-link')).toBeInTheDocument();
    expect(section.getByText('0')).toBeInTheDocument();
    expect(section.getByText('eth0 (link down)')).toBeInTheDocument();
    expect(section.getByText('medium')).toBeInTheDocument();
    expect(section.getAllByText('Not reported')).toHaveLength(3);
    expect(section.getByText('Routes observed', { exact: false })).toHaveTextContent(
      '2026-10-21T00:00:00Z',
    );
    expect(section.getByRole('table', { name: 'Main routing table routes' })).toBeInTheDocument();
  });

  it.each([' ETH1 ', '198.51.100', 'defAULT', '2001:DB8'])(
    'searches routes locally by %s with family filtering',
    async (term) => {
      render(<NetworkWorkspace host={host} />);
      await screen.findByText('192.0.2.1');
      const search = routeSection().getByRole('searchbox', { name: 'Search routes' });
      await userEvent.type(search, term);
      expect(routeSection().getByRole('status')).toHaveTextContent(
        term.trim().toLowerCase() === 'default' ? 'Showing 2 of 3 routes' : 'Showing 1 of 3 routes',
      );
      await userEvent.clear(search);
      await userEvent.selectOptions(
        routeSection().getByRole('combobox', { name: 'Route address family' }),
        'ipv6',
      );
      expect(routeSection().getByRole('status')).toHaveTextContent('Showing 1 of 3 routes');
      expect(routeSection().queryByText('192.0.2.1')).not.toBeInTheDocument();
      await userEvent.type(search, 'no-match');
      expect(routeSection().getByText('No routes match these filters.')).toBeInTheDocument();
      expect(mocks.list).toHaveBeenCalledTimes(1);
      expect(mocks.routes).toHaveBeenCalledTimes(1);
      expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    },
  );

  it.each(['ipv4', 'ipv6', 'empty'])(
    'explains missing families and absent defaults for %s inventory',
    async (family) => {
      const data = routeSnapshot();
      data.entries =
        family === 'empty'
          ? []
          : data.entries.filter(
              (entry) => entry.family === family && entry.destination !== 'default',
            );
      mocks.routes.mockResolvedValue(data);
      render(<NetworkWorkspace host={host} />);
      expect(
        await screen.findByText(
          'No default gateway is observed in the main table. This does not establish whether external connectivity is available.',
        ),
      ).toBeInTheDocument();
      if (!data.entries.some((entry) => entry.family === 'ipv4'))
        expect(
          routeSection().getByText('No IPv4 routes were returned from the main table.'),
        ).toBeInTheDocument();
      if (!data.entries.some((entry) => entry.family === 'ipv6'))
        expect(
          routeSection().getByText('No IPv6 routes were returned from the main table.'),
        ).toBeInTheDocument();
      if (!data.entries.length)
        expect(routeSection().getByText('No main-table routes were returned.')).toBeInTheDocument();
    },
  );

  it('keeps successful interfaces when routes fail initially, and retries only on manual Refresh', async () => {
    mocks.routes.mockRejectedValueOnce(new Error('Do not display remote failure details'));
    render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText('Route inventory is unavailable on this host.'),
    ).toBeInTheDocument();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    expect(screen.queryByText('Do not display remote failure details')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await screen.findByText('192.0.2.1')).toBeInTheDocument();
    expect(mocks.routes).toHaveBeenCalledTimes(2);
  });

  it('reads routes even when the preceding interface observation fails', async () => {
    mocks.list.mockRejectedValueOnce(new Error('Synthetic interface failure'));
    render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText('Network inventory is unavailable on this host.'),
    ).toBeInTheDocument();
    expect(await screen.findByText('192.0.2.1')).toBeInTheDocument();
    expect(mocks.routes).toHaveBeenCalledExactlyOnceWith(host.id, A);
  });

  it.each(['interfaces', 'routes', 'both'])(
    'preserves independent same-session snapshots when %s refresh fails',
    async (failure) => {
      render(<NetworkWorkspace host={host} />);
      await screen.findByText('192.0.2.1');
      if (failure !== 'routes') mocks.list.mockRejectedValueOnce(new Error('Synthetic failure'));
      if (failure !== 'interfaces')
        mocks.routes.mockRejectedValueOnce(new Error('Synthetic failure'));
      await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
      await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled());
      expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
      expect(routeSection().getByText('192.0.2.1')).toBeInTheDocument();
      if (failure !== 'interfaces')
        expect(
          routeSection().getByText(
            'Route refresh failed — showing the last successful same-session route snapshot.',
          ),
        ).toBeInTheDocument();
      if (failure !== 'routes')
        expect(
          screen.getByText('Refresh failed — showing the last successful snapshot.'),
        ).toBeInTheDocument();
    },
  );

  it.each([routeSnapshot(otherHost.id), routeSnapshot(host.id, B)])(
    'rejects wrong route ownership',
    async (data) => {
      mocks.routes.mockResolvedValueOnce(data);
      render(<NetworkWorkspace host={host} />);
      expect(
        await screen.findByText('Route inventory is unavailable on this host.'),
      ).toBeInTheDocument();
      expect(routeSection().queryByText('192.0.2.1')).not.toBeInTheDocument();
      expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    },
  );

  it('discards delayed routes on session replacement and never renders or requests old ownership again', async () => {
    const old = deferred<NetworkRouteSnapshot>();
    const fresh = deferred<NetworkRouteSnapshot>();
    mocks.routes.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
    const view = render(<NetworkWorkspace host={host} />);
    await waitFor(() => expect(mocks.routes).toHaveBeenCalledTimes(1));
    mocks.sessionId = B;
    mocks.list.mockResolvedValue(freshSnapshot(B));
    view.rerender(<NetworkWorkspace host={host} />);
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    await waitFor(() => expect(mocks.routes).toHaveBeenCalledTimes(2));
    await act(async () => {
      old.resolve(routeSnapshot());
    });
    expect(routeSection().queryByText('192.0.2.1')).not.toBeInTheDocument();
    const result = routeSnapshot(host.id, B);
    result.entries = [];
    await act(async () => {
      fresh.resolve(result);
    });
    expect(routeSection().getByText('No main-table routes were returned.')).toBeInTheDocument();
    expect(mocks.routes).toHaveBeenLastCalledWith(host.id, B);
  });

  it.each(['host', 'disconnect', 'unmount'])(
    'drops pending route responses after %s turnover',
    async (turnover) => {
      const old = deferred<NetworkRouteSnapshot>();
      mocks.routes.mockReturnValueOnce(old.promise);
      const view = render(<NetworkWorkspace host={host} />);
      await waitFor(() => expect(mocks.routes).toHaveBeenCalledTimes(1));
      if (turnover === 'unmount') view.unmount();
      else if (turnover === 'disconnect') {
        mocks.state = 'disconnected';
        mocks.sessionId = null;
        view.rerender(<NetworkWorkspace host={host} />);
      } else {
        mocks.list.mockResolvedValue(snapshot(otherHost.id));
        mocks.routes.mockResolvedValue({ ...routeSnapshot(otherHost.id), entries: [] });
        view.rerender(<NetworkWorkspace host={otherHost} />);
      }
      await act(async () => {
        old.resolve(routeSnapshot());
      });
      expect(screen.queryByText('192.0.2.1')).not.toBeInTheDocument();
      if (turnover !== 'host') expect(mocks.routes).toHaveBeenCalledTimes(1);
    },
  );

  it('does not start routes after an old address response settles following unmount', async () => {
    const old = deferred<NetworkSnapshot>();
    mocks.list.mockReturnValueOnce(old.promise);
    const view = render(<NetworkWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(1));
    view.unmount();
    await act(async () => {
      old.resolve(snapshot());
    });
    expect(mocks.routes).not.toHaveBeenCalled();
  });

  it('has one request sequence under StrictMode and keeps local keyboard controls from refetching', async () => {
    render(
      <StrictMode>
        <NetworkWorkspace host={host} />
      </StrictMode>,
    );
    await screen.findByText('192.0.2.1');
    const search = routeSection().getByRole('searchbox', { name: 'Search routes' });
    search.focus();
    await userEvent.keyboard('eth1');
    expect(routeSection().getByRole('status')).toHaveTextContent('Showing 1 of 3 routes');
    await userEvent.tab();
    expect(routeSection().getByRole('combobox', { name: 'Route address family' })).toHaveFocus();
    expect(mocks.list).toHaveBeenCalledTimes(1);
    expect(mocks.routes).toHaveBeenCalledTimes(1);
  });
});

function portSnapshot(hostId = host.id, hostSessionId = A): ListeningPortSnapshot {
  return {
    hostId,
    hostSessionId,
    observedAt: '2026-10-21T12:00:00Z',
    entries: [
      { protocol: 'tcp', family: 'ipv4', localAddress: '192.0.2.10', port: 8080, state: 'listen' },
      {
        protocol: 'udp',
        family: 'ipv6',
        localAddress: '2001:db8::1%eth0',
        port: 5353,
        state: 'unconn',
      },
      { protocol: 'tcp', family: 'unknown', localAddress: '*', port: 8443, state: 'listen' },
    ],
  };
}
function portSection() {
  return within(screen.getByRole('region', { name: 'Listening Ports' }));
}

function dnsSnapshot(hostId = host.id, hostSessionId = A): DnsConfigurationSnapshot {
  return {
    hostId,
    hostSessionId,
    observedAt: '2026-10-21T12:01:00Z',
    nameservers: [
      { address: '127.0.0.53', family: 'ipv4' },
      { address: '2001:db8::53', family: 'ipv6' },
    ],
    searchDirective: 'search',
    searchDomains: ['example.test', 'xn--bcher-kva.example'],
    options: ['edns0', 'ndots:5'],
    sortlist: [{ address: '192.0.2.0', family: 'ipv4', netmask: '255.255.255.0' }],
  };
}

function dnsSection() {
  return within(screen.getByRole('region', { name: 'DNS Configuration — `/etc/resolv.conf`' }));
}
describe('Listening Ports', () => {
  beforeEach(() => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    mocks.list.mockResolvedValue(snapshot());
    mocks.routes.mockResolvedValue(routeSnapshot());
    mocks.listeningPorts.mockResolvedValue(portSnapshot());
    mocks.dnsConfiguration.mockResolvedValue(dnsSnapshot());
  });
  it('runs exactly one address-route-socket-DNS sequence and keeps Refresh disabled through DNS', async () => {
    const addresses = deferred<NetworkSnapshot>();
    const routes = deferred<NetworkRouteSnapshot>();
    const ports = deferred<ListeningPortSnapshot>();
    const dns = deferred<DnsConfigurationSnapshot>();
    mocks.list.mockReturnValueOnce(addresses.promise);
    mocks.routes.mockReturnValueOnce(routes.promise);
    mocks.listeningPorts.mockReturnValueOnce(ports.promise);
    mocks.dnsConfiguration.mockReturnValueOnce(dns.promise);
    render(<NetworkWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A));
    expect(mocks.routes).not.toHaveBeenCalled();
    expect(mocks.listeningPorts).not.toHaveBeenCalled();
    expect(mocks.dnsConfiguration).not.toHaveBeenCalled();
    await act(async () => {
      addresses.resolve(snapshot());
    });
    expect(mocks.routes).toHaveBeenCalledExactlyOnceWith(host.id, A);
    expect(mocks.listeningPorts).not.toHaveBeenCalled();
    expect(mocks.dnsConfiguration).not.toHaveBeenCalled();
    await act(async () => {
      routes.resolve(routeSnapshot());
    });
    expect(mocks.listeningPorts).toHaveBeenCalledExactlyOnceWith(host.id, A);
    expect(mocks.dnsConfiguration).not.toHaveBeenCalled();
    const button = screen.getByRole('button', { name: 'Refresh' });
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(mocks.list).toHaveBeenCalledTimes(1);
    await act(async () => {
      ports.resolve(portSnapshot());
    });
    expect(mocks.dnsConfiguration).toHaveBeenCalledExactlyOnceWith(host.id, A);
    expect(button).toBeDisabled();
    await act(async () => {
      dns.resolve(dnsSnapshot());
    });
    expect(button).toBeEnabled();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    expect(screen.getByText('192.0.2.1')).toBeInTheDocument();
  });
  it('renders counts, explicit unknown family, reported state, timestamp and reachability limits', async () => {
    render(<NetworkWorkspace host={host} />);
    await screen.findByText('192.0.2.10');
    const section = portSection();
    expect(section.getByText('TCP listening: 2 · UDP bound sockets: 1')).toBeInTheDocument();
    expect(section.getByText('Unknown')).toBeInTheDocument();
    expect(section.getByText('*')).toBeInTheDocument();
    expect(section.getByText('UNCONN')).toBeInTheDocument();
    expect(section.getAllByText('LISTEN')).toHaveLength(2);
    expect(section.getByText('2001:db8::1%eth0')).toBeInTheDocument();
    expect(section.getByText('Listening ports observed', { exact: false })).toHaveTextContent(
      '2026-10-21T12:00:00Z',
    );
    expect(
      section.getByText(/does not prove external reachability or firewall exposure/),
    ).toHaveTextContent('UDP bound sockets do not prove');
    expect(section.getByRole('table', { name: 'Listening and bound sockets' })).toBeInTheDocument();
  });
  it.each(['192.0.2', ' 5353 ', 'UDP', 'UNCONN'])(
    'searches locally for %s, combines TCP/UDP filters and restores rows',
    async (term) => {
      render(<NetworkWorkspace host={host} />);
      await screen.findByText('192.0.2.10');
      const section = portSection();
      const search = section.getByRole('searchbox', { name: 'Search listening ports' });
      await userEvent.type(search, term);
      expect(section.getByRole('status')).toHaveTextContent('Showing 1 of 3 sockets');
      await userEvent.clear(search);
      const protocol = section.getByRole('combobox', { name: 'Socket protocol' });
      await userEvent.selectOptions(protocol, 'tcp');
      expect(section.getByRole('status')).toHaveTextContent('Showing 2 of 3 sockets');
      expect(section.getByText('Unknown')).toBeInTheDocument();
      await userEvent.selectOptions(protocol, 'udp');
      expect(section.getByRole('status')).toHaveTextContent('Showing 1 of 3 sockets');
      await userEvent.type(search, 'no-match');
      expect(section.getByText('No sockets match these filters.')).toBeInTheDocument();
      await userEvent.clear(search);
      await userEvent.selectOptions(protocol, 'all');
      expect(section.getByRole('status')).toHaveTextContent('Showing 3 of 3 sockets');
      expect(mocks.list).toHaveBeenCalledTimes(1);
      expect(mocks.routes).toHaveBeenCalledTimes(1);
      expect(mocks.listeningPorts).toHaveBeenCalledTimes(1);
    },
  );
  it('distinguishes empty socket success from unavailability and preserves the first two sections', async () => {
    mocks.listeningPorts.mockResolvedValueOnce({ ...portSnapshot(), entries: [] });
    const view = render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText('No supported listening or bound sockets were returned.'),
    ).toBeInTheDocument();
    expect(portSection().getByText('TCP listening: 0 · UDP bound sockets: 0')).toBeInTheDocument();
    view.unmount();
    mocks.listeningPorts.mockRejectedValueOnce(new Error('Private remote failure details'));
    render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText('Listening-port inventory is unavailable on this host.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('Private remote failure details')).not.toBeInTheDocument();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    expect(screen.getByText('192.0.2.1')).toBeInTheDocument();
    expect(
      screen.queryByText('No supported listening or bound sockets were returned.'),
    ).not.toBeInTheDocument();
  });
  it.each(['interfaces', 'routes', 'sockets', 'all'])(
    'keeps independent snapshots after %s Refresh failure',
    async (failure) => {
      render(<NetworkWorkspace host={host} />);
      await screen.findByText('192.0.2.10');
      if (failure === 'interfaces' || failure === 'all')
        mocks.list.mockRejectedValueOnce(new Error('Synthetic failure'));
      if (failure === 'routes' || failure === 'all')
        mocks.routes.mockRejectedValueOnce(new Error('Synthetic failure'));
      if (failure === 'sockets' || failure === 'all')
        mocks.listeningPorts.mockRejectedValueOnce(new Error('Synthetic failure'));
      await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
      await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled());
      expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
      expect(screen.getByText('192.0.2.1')).toBeInTheDocument();
      expect(portSection().getByText('192.0.2.10')).toBeInTheDocument();
      if (failure === 'sockets' || failure === 'all')
        expect(
          portSection().getByText(
            'Listening-port refresh failed — showing the last successful same-session socket snapshot.',
          ),
        ).toBeInTheDocument();
      expect(mocks.listeningPorts).toHaveBeenCalledTimes(2);
    },
  );
  it.each(['interfaces', 'routes', 'both'])(
    'still observes sockets when earlier %s initial reads fail',
    async (failure) => {
      if (failure !== 'routes') mocks.list.mockRejectedValueOnce(new Error('Synthetic failure'));
      if (failure !== 'interfaces')
        mocks.routes.mockRejectedValueOnce(new Error('Synthetic failure'));
      render(<NetworkWorkspace host={host} />);
      expect(await screen.findByText('192.0.2.10')).toBeInTheDocument();
      expect(mocks.listeningPorts).toHaveBeenCalledExactlyOnceWith(host.id, A);
      expect(mocks.dnsConfiguration).toHaveBeenCalledExactlyOnceWith(host.id, A);
    },
  );
  it.each([portSnapshot(otherHost.id), portSnapshot(host.id, B)])(
    'rejects mismatched socket ownership',
    async (data) => {
      mocks.listeningPorts.mockResolvedValueOnce(data);
      render(<NetworkWorkspace host={host} />);
      expect(
        await screen.findByText('Listening-port inventory is unavailable on this host.'),
      ).toBeInTheDocument();
      expect(portSection().queryByText('192.0.2.10')).not.toBeInTheDocument();
    },
  );
  it('clears visible sockets on disconnect and reconnects with a fresh session', async () => {
    const view = render(<NetworkWorkspace host={host} />);
    await screen.findByText('192.0.2.10');
    mocks.state = 'disconnected';
    mocks.sessionId = null;
    view.rerender(<NetworkWorkspace host={host} />);
    expect(screen.queryByText('192.0.2.10')).not.toBeInTheDocument();
    mocks.state = 'connected';
    mocks.sessionId = B;
    mocks.list.mockResolvedValue(freshSnapshot(B));
    mocks.routes.mockResolvedValue(routeSnapshot(host.id, B));
    const next = deferred<ListeningPortSnapshot>();
    mocks.listeningPorts.mockReturnValueOnce(next.promise);
    view.rerender(<NetworkWorkspace host={host} />);
    expect(screen.queryByText('192.0.2.10')).not.toBeInTheDocument();
    await waitFor(() => expect(mocks.listeningPorts).toHaveBeenCalledTimes(2));
    await act(async () => {
      next.resolve({ ...portSnapshot(host.id, B), entries: [] });
    });
    expect(
      portSection().getByText('No supported listening or bound sockets were returned.'),
    ).toBeInTheDocument();
    expect(mocks.listeningPorts).toHaveBeenLastCalledWith(host.id, B);
  });
  it.each(['host', 'session', 'disconnect', 'unmount'])(
    'drops delayed socket responses after %s turnover',
    async (turnover) => {
      const old = deferred<ListeningPortSnapshot>();
      mocks.listeningPorts.mockReturnValueOnce(old.promise);
      const view = render(<NetworkWorkspace host={host} />);
      await waitFor(() => expect(mocks.listeningPorts).toHaveBeenCalledTimes(1));
      if (turnover === 'unmount') view.unmount();
      else if (turnover === 'disconnect') {
        mocks.state = 'disconnected';
        mocks.sessionId = null;
        view.rerender(<NetworkWorkspace host={host} />);
      } else {
        const nextHost = turnover === 'host' ? otherHost : host;
        const nextSession = turnover === 'session' ? B : A;
        mocks.sessionId = nextSession;
        mocks.list.mockResolvedValue(snapshot(nextHost.id, nextSession));
        mocks.routes.mockResolvedValue(routeSnapshot(nextHost.id, nextSession));
        mocks.listeningPorts.mockResolvedValue({
          ...portSnapshot(nextHost.id, nextSession),
          entries: [],
        });
        view.rerender(<NetworkWorkspace host={nextHost} />);
      }
      await act(async () => {
        old.resolve(portSnapshot());
      });
      expect(screen.queryByText('192.0.2.10')).not.toBeInTheDocument();
    },
  );
  it('does not start the socket read after an old route response settles on unmount', async () => {
    const old = deferred<NetworkRouteSnapshot>();
    mocks.routes.mockReturnValueOnce(old.promise);
    const view = render(<NetworkWorkspace host={host} />);
    await waitFor(() => expect(mocks.routes).toHaveBeenCalledTimes(1));
    view.unmount();
    await act(async () => {
      old.resolve(routeSnapshot());
    });
    expect(mocks.listeningPorts).not.toHaveBeenCalled();
  });
  it('has one four-section sequence under StrictMode and keyboard filters cause no remote calls', async () => {
    render(
      <StrictMode>
        <NetworkWorkspace host={host} />
      </StrictMode>,
    );
    await screen.findByText('192.0.2.10');
    portSection().getByRole('searchbox', { name: 'Search listening ports' }).focus();
    await userEvent.keyboard('8443');
    await userEvent.tab();
    expect(portSection().getByRole('combobox', { name: 'Socket protocol' })).toHaveFocus();
    expect(portSection().getByRole('status')).toHaveTextContent('Showing 1 of 3 sockets');
    expect(mocks.list).toHaveBeenCalledTimes(1);
    expect(mocks.routes).toHaveBeenCalledTimes(1);
    expect(mocks.listeningPorts).toHaveBeenCalledTimes(1);
    expect(mocks.dnsConfiguration).toHaveBeenCalledTimes(1);
  });
});

describe('DNS Configuration', () => {
  beforeEach(() => {
    mocks.state = 'connected';
    mocks.sessionId = A;
    mocks.list.mockResolvedValue(snapshot());
    mocks.routes.mockResolvedValue(routeSnapshot());
    mocks.listeningPorts.mockResolvedValue(portSnapshot());
    mocks.dnsConfiguration.mockResolvedValue(dnsSnapshot());
  });

  it('renders validated directives, source, timestamp and explicit observation limits as text', async () => {
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('127.0.0.53')).toBeInTheDocument();
    const section = dnsSection();
    expect(section.getByText('2001:db8::53')).toBeInTheDocument();
    expect(section.getAllByText('IPv4')).toHaveLength(1);
    expect(section.getAllByText('IPv6')).toHaveLength(1);
    expect(section.getByText(/Observed search directive:/)).toHaveTextContent(
      'example.test, xn--bcher-kva.example',
    );
    expect(section.getByText('edns0, ndots:5')).toBeInTheDocument();
    expect(section.getByText(/192\.0\.2\.0\/255\.255\.255\.0 \(IPv4\)/)).toBeInTheDocument();
    expect(section.getByText('DNS configuration observed', { exact: false })).toHaveTextContent(
      '2026-10-21T12:01:00Z',
    );
    expect(section.getByText(/127\.0\.0\.53 nameserver can identify/)).toHaveTextContent(
      'does not reveal upstream resolvers',
    );
    expect(section.getByText(/Per-link and DHCP DNS/)).toHaveTextContent(
      'does not test DNS reachability, health, response correctness or security',
    );
    expect(document.querySelector('img')).toBeNull();
  });

  it('distinguishes a valid empty snapshot from failure without claiming DNS is disabled', async () => {
    mocks.dnsConfiguration.mockResolvedValueOnce({ ...dnsSnapshot(), nameservers: [] });
    const view = render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText(
        'No nameserver directives were observed. This does not prove that DNS is disabled.',
      ),
    ).toBeInTheDocument();
    view.unmount();
    mocks.dnsConfiguration.mockRejectedValueOnce(new Error('PRIVATE HOST OUTPUT'));
    render(<NetworkWorkspace host={host} />);
    expect(
      await screen.findByText('DNS configuration is unavailable on this host.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('PRIVATE HOST OUTPUT')).not.toBeInTheDocument();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    expect(screen.getByText('192.0.2.10')).toBeInTheDocument();
  });

  it('retains only a same-session DNS snapshot with an explicit stale warning after Refresh failure', async () => {
    render(<NetworkWorkspace host={host} />);
    await screen.findByText('127.0.0.53');
    mocks.dnsConfiguration.mockRejectedValueOnce(new Error('Synthetic DNS failure'));
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(
      await screen.findByText(
        'DNS configuration refresh failed — showing the last successful same-session snapshot.',
      ),
    ).toBeInTheDocument();
    expect(dnsSection().getByText('127.0.0.53')).toBeInTheDocument();
    expect(mocks.dnsConfiguration).toHaveBeenCalledTimes(2);
  });

  it('still reads DNS after every earlier Network section fails', async () => {
    mocks.list.mockRejectedValueOnce(new Error('interfaces'));
    mocks.routes.mockRejectedValueOnce(new Error('routes'));
    mocks.listeningPorts.mockRejectedValueOnce(new Error('ports'));
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('127.0.0.53')).toBeInTheDocument();
    expect(mocks.dnsConfiguration).toHaveBeenCalledExactlyOnceWith(host.id, A);
  });

  it.each([dnsSnapshot(otherHost.id), dnsSnapshot(host.id, B)])(
    'rejects mismatched DNS ownership',
    async (data) => {
      mocks.dnsConfiguration.mockResolvedValueOnce(data);
      render(<NetworkWorkspace host={host} />);
      expect(
        await screen.findByText('DNS configuration is unavailable on this host.'),
      ).toBeInTheDocument();
      expect(dnsSection().queryByText('127.0.0.53')).not.toBeInTheDocument();
    },
  );

  it.each(['host', 'session', 'disconnect', 'unmount'])(
    'drops delayed DNS responses after %s turnover',
    async (turnover) => {
      const old = deferred<DnsConfigurationSnapshot>();
      mocks.dnsConfiguration.mockReturnValueOnce(old.promise);
      const view = render(<NetworkWorkspace host={host} />);
      await waitFor(() => expect(mocks.dnsConfiguration).toHaveBeenCalledTimes(1));
      if (turnover === 'unmount') view.unmount();
      else if (turnover === 'disconnect') {
        mocks.state = 'disconnected';
        mocks.sessionId = null;
        view.rerender(<NetworkWorkspace host={host} />);
      } else {
        const nextHost = turnover === 'host' ? otherHost : host;
        const nextSession = turnover === 'session' ? B : A;
        mocks.sessionId = nextSession;
        mocks.list.mockResolvedValue(snapshot(nextHost.id, nextSession));
        mocks.routes.mockResolvedValue(routeSnapshot(nextHost.id, nextSession));
        mocks.listeningPorts.mockResolvedValue(portSnapshot(nextHost.id, nextSession));
        mocks.dnsConfiguration.mockResolvedValue({
          ...dnsSnapshot(nextHost.id, nextSession),
          nameservers: [],
        });
        view.rerender(<NetworkWorkspace host={nextHost} />);
      }
      await act(async () => {
        old.resolve(dnsSnapshot());
      });
      expect(screen.queryByText('127.0.0.53')).not.toBeInTheDocument();
    },
  );

  it('starts exactly one four-section sequence in StrictMode and prevents overlapping Refresh', async () => {
    const dns = deferred<DnsConfigurationSnapshot>();
    mocks.dnsConfiguration.mockReturnValueOnce(dns.promise);
    render(
      <StrictMode>
        <NetworkWorkspace host={host} />
      </StrictMode>,
    );
    await waitFor(() => expect(mocks.dnsConfiguration).toHaveBeenCalledTimes(1));
    const refresh = screen.getByRole('button', { name: 'Refresh' });
    expect(refresh).toBeDisabled();
    await userEvent.click(refresh);
    expect(mocks.list).toHaveBeenCalledTimes(1);
    expect(mocks.routes).toHaveBeenCalledTimes(1);
    expect(mocks.listeningPorts).toHaveBeenCalledTimes(1);
    expect(mocks.dnsConfiguration).toHaveBeenCalledTimes(1);
    await act(async () => {
      dns.resolve(dnsSnapshot());
    });
    expect(refresh).toBeEnabled();
  });
});

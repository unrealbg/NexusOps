# Read-only network interface and address inventory

Network observes the selected Linux host through its current verified SSH session. It requests exactly one compile-time fixed read on workspace entry and on manual Refresh:

```sh
LC_ALL=C ip -j address show
```

The renderer supplies only `HostId` and `HostSessionId`; it cannot choose an interface, family, address, filter, command or extra argument. `iproute2` must be available to the remote account. This observation requests no sudo or polkit and does not change remote network state. A hostile SSH account or server can redefine `ip` or lie about its output, so the result is not an attestation.

The parser consumes `ifindex`, `ifname` and `addr_info`, plus optional `mtu` and `operstate`. Each address must have exactly one family representation: textual `family` or numeric `family_index`, which iproute2 uses for unrecognized families. From recognized `inet` or `inet6` entries it consumes `family`, `local` and `prefixlen`; numeric and other unsupported families are omitted but still count toward the raw entry limit. Link-layer `address`, flags, lifetimes, labels and other iproute2 properties are ignored and never published. IPv4 and IPv6 values are validated and canonicalized with Rust `IpAddr`; family mismatches, invalid prefixes, duplicate interfaces or addresses, malformed JSON and deceptive display text fail with one fixed safe error. A missing state or MTU remains unknown, and an unfamiliar bounded state is displayed neutrally.

One operation has an 8-second deadline and a 64 KiB combined-output limit. The parser permits at most 128 interfaces and 512 raw address entries, with 64-byte interface names, 32-byte state tokens and 64-byte raw IP strings. Oversized or malformed results are unavailable, not partial success. The SSH transport does not expose arbitrary stderr to the UI, so unavailable does not claim a particular cause.

Core requires the exact host/session pair, captures the connection generation and cancellation token, then revalidates them after SSH I/O. Requests have non-queuing admission: one per host and four globally, independent of Services admission. Disconnect, reconnect, remote closure, edit, delete and shutdown prevent stale publication. Host deletion and shutdown remove network admission gates. The Network component checks response identity again, ignores delayed old-owner responses and unmounts, and keeps its snapshot, error, filter and timestamp in component-local memory only. It makes no background request or refetch-on-focus; filtering is local. A failed Refresh can retain only the last successful snapshot from the same session.

Interface names and IP addresses are sensitive topology metadata. Neither raw output nor parsed values enter SQLite, browser storage, Query, Zustand, application logs, audit payloads or published evidence. React renders selected values as text only. This workspace does not expose routes, gateways, sockets, DNS settings, MAC addresses, firewall rules, interface controls, packet contents or traffic history.

Deterministic parser, policy, core lifecycle, component and loopback SSH tests cover the source behavior. Real `iproute2` interoperability and Native Windows acceptance remain separate post-source-review gates for an immutable production candidate.

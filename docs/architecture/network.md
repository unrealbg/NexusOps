# Read-only network interfaces, main-table routes and listening ports

Network observes the selected Linux host through its current verified SSH session. Its interface section requests one compile-time fixed read on workspace entry and on manual Refresh:

```sh
LC_ALL=C ip -j address show
```

The renderer supplies only `HostId` and `HostSessionId`; it cannot choose an interface, family, address, filter, command or extra argument. `iproute2` must be available to the remote account. This observation requests no sudo or polkit and does not change remote network state. A hostile SSH account or server can redefine `ip` or lie about its output, so the result is not an attestation.

The parser consumes `ifindex`, `ifname` and `addr_info`, plus optional `mtu` and `operstate`. Each address must have exactly one family representation: textual `family` or numeric `family_index`, which iproute2 uses for unrecognized families. From recognized `inet` or `inet6` entries it consumes `family`, `local` and `prefixlen`; numeric and other unsupported families are omitted but still count toward the raw entry limit. Link-layer `address`, flags, lifetimes, labels and other iproute2 properties are ignored and never published. IPv4 and IPv6 values are validated and canonicalized with Rust `IpAddr`; family mismatches, invalid prefixes, duplicate interfaces or addresses, malformed JSON and deceptive display text fail with one fixed safe error. A missing state or MTU remains unknown, and an unfamiliar bounded state is displayed neutrally.

One operation has an 8-second deadline and a 64 KiB combined-output limit. The parser permits at most 128 interfaces and 512 raw address entries, with 64-byte interface names, 32-byte state tokens and 64-byte raw IP strings. Oversized or malformed results are unavailable, not partial success. The SSH transport does not expose arbitrary stderr to the UI, so unavailable does not claim a particular cause.

Core requires the exact host/session pair, captures the connection generation and cancellation token, then revalidates them after SSH I/O. Requests have non-queuing admission: one per host and four globally, independent of Services admission. Disconnect, reconnect, remote closure, edit, delete and shutdown prevent stale publication. Host deletion and shutdown remove network admission gates. The Network component checks response identity again, ignores delayed old-owner responses and unmounts, and keeps its snapshot, error, filter and timestamp in component-local memory only. It makes no background request or refetch-on-focus; filtering is local. A failed Refresh can retain only the last successful snapshot from the same session.

Interface names and IP addresses are sensitive topology metadata. Neither raw output nor parsed values enter SQLite, browser storage, Query, Zustand, application logs, audit payloads or published evidence. React renders selected values as text only. This workspace does not expose DNS settings, MAC addresses, firewall rules, interface controls, packet contents or traffic history. Listening/bound sockets are observed separately as documented below. The separately observed Routes & Gateway section is bounded below.

Deterministic parser, policy, core lifecycle, component and loopback SSH tests cover the source behavior. Real `iproute2` interoperability and Native Windows acceptance passed for source-reviewed HEAD `de0776df24b70d2ed2987789d2441f13ecc4d191`; the P2 `family_index` finding is resolved. Acceptance used the unchanged production executable (18,598,912 bytes; SHA-256 `D963EE10FAE73126845705A2719B216F787564BE81E0A4E38188CC4A46D165A3`). The [PR #8 acceptance evidence](https://github.com/unrealbg/NexusOps/pull/8#issuecomment-5854656263) records the completed checks; the acceptance-evidence JSON has SHA-256 `80B55DAD39811A0E371CC1FED9441187A1110B58C7E3C28F95951CC640EA5683`.

Cleanup remains **PARTIAL — POLICY-BLOCKED OFFLINE LEFTOVERS**. The runtime fixture was fully removed. The remaining offline artifacts are accepted local cleanup debt and do not change the product acceptance result.

## Goal 06B: Routes & Gateway

`list_host_routes(HostId, HostSessionId)` is one dedicated typed read-only IPC command. The renderer supplies no route arguments. Both required reads use the unchanged OperationEngine policy, eight-second deadline and 64 KiB output bound **per command**, without elevation, retries or interpolation:

```sh
LC_ALL=C ip -j -4 route show table main
LC_ALL=C ip -j -6 route show table main
```

One logical observation retains the existing Network one-per-host/four-global permits across both sequential commands and parsing. Addresses and routes share these permits. Any failed family read, unsupported row, cancellation, malformed payload or combined count above 128 makes the entire route snapshot unavailable. No partial inventory is published. Core revalidates connection generation, cancellation, state and both session identity representations after I/O.

The parser accepts simple unicast routes with a required bounded ASCII interface token and validated same-family destination/gateway. Host routes become canonical `/32` or `/128` destinations; zero prefixes become `default`. Prefixes with nonzero host bits and duplicate canonical route identities (destination, gateway, interface, optional metric) fail closed. A missing metric remains absent, including when other routes report zero. Distinct default observations remain separate; neither sorting nor metrics invent a uniquely preferred gateway. Gateway absence is displayed as direct/on-link; explicit onlink/linkdown flags and IPv6 router preference are preserved and displayed.

Selected JSON fields are strictly typed and duplicate checked; present nulls are invalid. The allowlist is `dst`, `dev`, `gateway`, `metric`, `type`, `protocol`, `scope`, `prefsrc`, `table`, `flags`, `pref`. Only absent/unicast type, absent/main/254 table, global/universe/link/host scope, onlink/linkdown flags, and low/medium/high IPv6 preference are accepted. Protocol is a bounded ASCII token or number 0–255; preferred-source addresses must match the family. Protocol, scope, preferred source and table are validated metadata, not published. Unknown fields, multipath/nexthops, nexthop IDs, encapsulation, cross-family via, source-specific routes, TOS selectors and extra route metrics fail closed rather than being flattened. Consequently some valid but unsupported iproute2 responses are unavailable. Interface tokens use ASCII letters/digits and `_ - . : @`, at most 64 bytes; raw IP strings are at most 64 bytes. Each parser input is also bounded to 64 KiB.

This observes the **main routing table only**, not every table, policy rule, VRF or the route selected for a particular packet. No observed default route does not prove a lack of external connectivity. Reported rows remain observations from the remote account, not an attestation of kernel state or connectivity.

Network entry and manual Refresh coordinate interface then route requests sequentially, even when interfaces fail. Each section has independent component-local success, pending, error and observation timestamp. Failed Refresh retains only that section's prior same-session snapshot. Changing host/session, disconnecting or unmounting discards the owned component and prevents late results and subsequent old-owner route requests. StrictMode's discarded setup starts no IPC. There is no polling, focus refetch, topology cache, persistence, Copy/Export, route editing or connectivity testing. Search and family filters are local.

Goal 06B validation is synthetic and compilation-only. The historical PR #8 runtime acceptance above applies to interfaces/addresses, **not** these new route reads. Real SSH interoperability, Linux main-table behavior and native application acceptance for routes remain NOT RUN. No G26/G28 authority or PR #35 state changes are included.

## Goal 06C: Listening Ports

The dedicated `list_host_listening_ports(HostId, HostSessionId)` operation admits exactly one fixed, read-only command through the unchanged OperationEngine and verified transport:

```sh
LC_ALL=C ss -H -lntu
```

There are no renderer-supplied arguments, retries, fallbacks, elevation, process/PID/username inspection, scans, connection attempts or writes. This command is explicitly rejected by connection discovery probes. Its eight-second deadline and 64 KiB combined-output limit are unchanged. The typed Tauri command retains exactly one native lifecycle permit; the generated allow/deny permission and main-window allow permission are specific to this command. The dynamic lifecycle verifier remains unchanged, and the independently confirmed command count is 61.

The parser accepts only six headerless columns: protocol, state, receive queue, send queue, local endpoint and peer endpoint. Protocol/state pairs must be TCP/LISTEN or UDP/UNCONN. Queue values are unsigned decimal u64, at most 20 digits, and are discarded. Local ports are decimal 1–65535. Peer ports must be `*` or `0`, with only wildcard/unspecified peer addresses; peer values are validated and discarded. Connected UDP, non-listening TCP, process columns, extended fields, headers, address-family mismatches and unsupported forms reject the whole inventory with one fixed safe error.

Every input is bounded to 65,536 UTF-8 bytes, every line to 512 bytes (excluding LF/CRLF), each endpoint to 128 bytes, raw IP text to 64 bytes, and IPv6 interface zones to 32 ASCII letters/digits/underscore/hyphen/dot. Numeric zones must be nonzero u32 and are canonicalized. Queue fields and fixed protocol/state tokens have explicit bounds. Only ASCII printing text and delimiter tabs/LF/CRLF are accepted; ANSI escapes, other controls, unpaired carriage returns, Unicode controls and deceptive display text fail closed. At most 256 socket rows are accepted. Blank lines are ignored within the total-output bound; empty valid output is successful and distinct from transport/parser unavailability.

IPv4, bracketed IPv6 and rightmost-colon-delimited unbracketed IPv6 endpoints are validated with Rust IpAddr and canonicalized. IPv6 zones are retained as validated address suffixes. A bare `*` does not establish address family: `*:port *:*` remains explicitly Unknown. A syntactically explicit IPv4/IPv6 unspecified peer can establish the wildcard's family; then the bind address is canonicalized to that family's unspecified IP. Bracketed IPv4/wildcard forms and inconsistent explicit families are rejected. Canonical identity includes protocol, family, local address (with zone), port and validated state. Equivalent IP/port/zone spellings or duplicate binds without distinguishing allowed fields reject the entire result. Distinct protocols, addresses, scopes or ports remain separate; no hidden process identity is invented to distinguish reuseport sockets.

Socket observations reuse the same non-queuing Network admission as interfaces and routes: one active observation per host and four globally. Host/session, cancellation and connection generation are revalidated after I/O. Failures, cancellation and dropped futures release their RAII permits. Services, terminal and SFTP authority are not changed. No network topology snapshot is persisted or placed in Query/Zustand, logs or audit content.

The mounted connected Network workspace reads interfaces, then routes, then listening ports, including when an earlier section fails. Each section keeps its own successful snapshot, timestamp, pending state and error in component-local memory. Refresh cannot overlap; a failure retains only a successful snapshot from the same session with an explicit stale warning. Keyed ownership and generation guards clear data on host/session change or disconnect, discard late results after unmount, and suppress discarded StrictMode setup requests. Search and TCP/UDP filtering are entirely local, with Unknown family displayed explicitly. No polling, focus refetch, Copy, Export or persistence is added.

TCP listening and UDP bound counts describe observations only. A locally listening socket does not establish external reachability or firewall exposure, and a UDP bound socket does not prove a reachable service is accepting traffic. The remote account can substitute `ss` or provide false output; the snapshot is not an attestation. Unsupported ss variations and indistinguishable duplicate socket identities fail closed. Real-host/SSH interoperability and native runtime acceptance for this feature are NOT RUN and remain separately unauthorized. Historical interface/route evidence does not establish listening-port acceptance. G26, G28 and PR #35 holds remain unchanged.

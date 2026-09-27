# SSH endpoint trust inventory

Security is a read-only view of local SSH endpoint trust and configured authentication metadata. It works for an offline selected host. It is not a credential checker, vulnerability scan, security score, compliance verdict, or attestation of the remote host or local OS.

## Local authority and ownership

The single `get_host_ssh_trust(HostId)` command resolves the current host record in core; the renderer cannot supply an endpoint, fingerprint, credential, database path, or session ID. `SshEndpointTrust` contains only the host ID, configured hostname, port, authentication method and optional endpoint pin. It does not duplicate `HostSession`.

Core holds the existing metadata mutation gate while loading and validating the stored connection configuration and reading `KnownHosts::fingerprint()`. This serializes the snapshot with host edits, deletion and trust acceptance using the existing lock order. These are bounded local reads, with no network I/O under the gate. Corrupted host configuration returns a fixed persistence error. Stored pin algorithm and SHA-256 values must pass the existing bounded validation; malformed, oversized, control-character or bidi data returns a fixed storage error without echoing the value. An invalid/unreadable pin is not an absent pin.

Pins belong to the **canonical hostname plus SSH port**, not to a `HostId`. Two host records using the same endpoint share its pin. Deleting Host A does not delete that pin; a later Host B for the same endpoint can correctly display it. A different hostname alias or port has separate pin state, even if it reaches the same physical server. Editing a host to endpoint B immediately invalidates endpoint A's renderer snapshot; A's pin may remain in local storage.

Configured authentication displays only **Password** or **Private key**, taken from ordinary host metadata. It makes no assertion about credential presence, validity, availability or keychain state. The getter calls no `SecretStore` method and returns no credential revision identifier or value.

Security adds no remote commands, SSH exec operations, connection attempts, sudo/polkit, or remote mutations. It does not reconcile sessions. There are no Trust, Copy, Export, Replace, Rotate or Delete-pin actions, and no new clipboard or filesystem capability. Existing Overview remains the connection and first-contact trust surface. Changed-key blocking and certificate rejection are unchanged.

## Session meaning

The existing `useHostSession(host.id)` query is the sole connection-state source. The transport's `check_server_key()` verifies the actual presented raw host key against the endpoint pin before authentication. After successful connection and discovery, `Application::establish()` reads the local pin again and records it in `HostSession.identity`. That identity is session-associated local pin metadata recorded after a verified connect; it is not a separately captured handshake fingerprint measurement.

Security shows **Verified against endpoint pin at session establishment** only when the current session is Connected, its host ID matches, its `HostSessionId` and identity are present, an endpoint pin is present, and identity hostname, algorithm and SHA-256 exactly match the local trust snapshot. A Connected session with missing or inconsistent metadata shows verification unavailable. No fingerprint is described as a live or fresh remote observation.

Disconnected, Disconnecting and Failed states show **No active verified session**, even if `HostSession.identity` remains populated. This matters when existing `get_session()` reconciliation detects remote closure and retains identity while transitioning to Failed. Connecting shows verification in progress. AwaitingTrust shows awaiting first-contact trust, separately from the endpoint's Not pinned state. A changed-key error explains that the connection was blocked without displaying the challenged key or offering replacement.

Session-query errors hide any retained query data and show session state unavailable; they do not imply Disconnected or Verified. A successful offline pin read means **Pinned locally**, never a fresh remote observation.

## Renderer lifetime and persistence

Endpoint trust is component-local. Each request belongs to a host ID, hostname, port and authentication method, with a generation guard. Responses must match all four current fields. Configuration changes remount the local trust view, so returning to an earlier configuration still requires a fresh read. Delayed old-owner/unmounted results are ignored. Initial reads and manual Refresh are the only pin reads; failed reads/refreshes remove the prior snapshot and show unavailable, never Not pinned.

There is **no background polling of endpoint-pin metadata**. Existing host-session polling is reused unchanged for connection truthfulness and remote-closure detection. Session updates alone do not trigger a pin read.

No new tables, browser storage, Security history or persisted snapshots are introduced. Existing `hosts.db` and `known-hosts.db` supply local metadata; the existing credential vault is not read by this path. Trust snapshots do not enter Query, Zustand, logs or audit. Reads add no audit event and do not log endpoints, authentication methods, algorithms or fingerprints. React renders values as text; IPv6 endpoints use `[address]:port`.

## Validation and limits

Deterministic backend tests cover endpoint ownership, absence versus invalid storage, secret/transport isolation, no audit writes, delete/recreate, endpoint edit and serialized edit/trust/delete reads. Component tests cover all session states, metadata mismatches, remote closure, query/refresh errors, request ownership, unmount, manual refresh, text rendering and navigation. Production SSH loopback tests retain unknown-before-auth and changed-key rejection coverage.

Authority coverage is layered without adding a crate dependency: core exercises the actual application getter with panic-on-use secret/provider/transport doubles, while the existing production SSH loopback suite records connection/authentication/exec counters around the same `KnownHosts::fingerprint()` accessor after verified connect. The latter does not invoke the core facade itself. A separate handler regression rejects host certificates even when their raw subject key is pinned.

A malicious local account able to modify profile storage can tamper with well-formed host or pin metadata; syntax validation cannot attest its authenticity. The view offers no security/compliance score. Native Windows acceptance remains pending separate source review of an immutable implementation head.

# SSH integration tests

Run `cargo test -p nexus-ssh` for the provider unit tests and real SSH loopback integration tests. Run only the protocol fixture with `cargo test -p nexus-ssh --test loopback`.

The fixture binds ephemeral `127.0.0.1` ports and generates temporary Ed25519 host and user keys in memory. It uses the same russh protocol implementation as the client, authenticates a synthetic password or private key, and returns deterministic Linux probe text. It never starts a shell, invokes a local command, contacts an external server, writes a remote file or installs software. The regular test suite needs no VPS, Docker, OpenSSH installation, keychain unlock or administrator privileges.

Coverage includes:

- An unknown host aborts the real handshake before the server receives authentication; explicit trust permits reconnection and all eight discovery probes.
- A previously pinned endpoint presenting another key is rejected before authentication.
- Password success/rejection, private-key authentication, encrypted private-key authentication and incorrect passphrase rejection.
- Independent hosts retain isolated lifetimes.
- The combined stdout/stderr budget is 64 KiB per command; excessive output and failed optional probes preserve a usable session.
- Cancelling an operation closes its SSH channel; cancelling the connection lifetime invalidates the session. Unit tests separately verify cancellation wakes a pending TCP read and rejects work before it begins.
- SQLite pins survive reopening, bind hostname/port/algorithm, and cannot be replaced by a stale trust challenge.

This fixture validates the actual protocol and application boundaries, but is not an independent SSH interoperability implementation. Goal 01A therefore adds a separate, ignored test against an actual OpenSSH server in a marker-owned disposable Alpine WSL2 VM. Run `tools/openssh-fixture/Run-OpenSshInterop.ps1` after fixture setup; it covers password authentication, unencrypted and encrypted Ed25519 keys, trust ordering, persistence, real discovery, key rotation, independent sessions, reconnect, and a stalled handshake. The default suite remains self-contained. IPv6, a dropped real network, and slow individual discovery probes remain future interoperability checks; see [the OpenSSH record](../validation/openssh-interoperability.md).

Goal 02A extends the same combined run with a production-path PTY test. It requests `xterm-256color`, verifies UTF-8 and ANSI bytes, propagates `stty size`, isolates two shell channels, closes one independently, processes 2 MiB through the bounded queue, runs `top`, and tears terminals down on disconnect. The current fixture name and ownership marker are Goal 02A-specific; setup and cleanup refuse an existing or unmarked distribution.

## Manual desktop fixture

To exercise the real desktop application against a local SSH server, run:

```sh
cargo run -p nexus-ssh --example loopback_fixture
```

This example is test infrastructure, excluded from the shipped application. It always binds `127.0.0.1` and selects an ephemeral port. It prints one JSON line containing `testOnly`, `hostname`, `port`, `username`, `password`, `algorithm`, `fingerprint`, `pid` and `lifetimeSeconds`. The deliberately public, disposable credentials are username `nexusops` and password `fixture-only`. Only the eight fixed discovery commands are accepted; shells and all other commands are rejected. No received input is executed.

Create a desktop host with the announced address and port and password authentication. Connect, compare the trust dialog fingerprint with the JSON announcement, explicitly trust it, then connect again. The overview should show `nexus-fixture`, `Fixture Linux 1`, kernel `6.12.0-fixture`, load `0.25` and the fixture memory/filesystem values. The desktop still uses the actual OS credential vault and persistent host-key store.

The fixture exits after one hour. Stop its process earlier when finished. Its PID is in the announcement; in PowerShell use `Stop-Process -Id <fixture-pid>`. Windows locks running executable files, so stop the fixture before rebuilding it or running a full Cargo test that rebuilds examples. Alternatively, run a copy of the built fixture executable from a scratch directory. For a shorter run or a specific port:

```sh
cargo run -p nexus-ssh --example loopback_fixture -- --port 22222 --lifetime-seconds 900
```

Every launch generates a fresh host key in memory. To verify changed-key rejection, stop the fixture and restart it with `--port` set to the previous announced port. Reconnect the existing desktop host: the connection must be blocked as a changed host key. For a new trust test without maintaining local pins, launch on another ephemeral port. Remove disposable desktop host records after the test.

## Transport limits and current scope

DNS lookup, TCP connection and SSH handshake share a 15-second deadline. Authentication has a 30-second deadline. A transport command has a 10-second deadline and channel cleanup has a 2-second deadline; the operation engine applies its tighter application deadline. Keepalive probes run after 20 seconds of inactivity and close a peer that misses three responses. Packet size, channel queue and SSH flow-control windows are bounded in addition to the output collector.

Cancellation is enforced at the TCP stream, independently of russh's background task. Dropping a connection attempt or the last provider-session owner cancels that stream. A command channel guard sends bounded protocol cleanup even when an application caller drops its execution future. Disconnect always cancels local I/O even if the peer does not respond.

Raw host keys are supported; OpenSSH host certificates, SSH agents, keyboard-interactive authentication, proxies, bastions, forwarding, shells and SFTP are outside Goal 01. Host-key rotation has no bypass in the trust API: independently verify the change and deliberately maintain the local pin store before reconnecting. Private-key decoding runs on a blocking worker so it does not block the UI or Tokio executor. A decode already executing cannot be preempted by Tokio cancellation; credentials and imported keys are limited to 64 KiB, and any connection awaiting it is still cancelled at the transport boundary.

The pinned stable russh release uses its maintained Ring backend, with unused compression and optional RSA support disabled. Ed25519 is the supported and independently verified client-key type for the Windows alpha. Cargo.lock records the resolved dependency graph, including upstream pinned cryptography release candidates used by this stable russh release. Avoid logging raw russh errors: remote protocol fields can contain untrusted text. Provider diagnostics record operation stage, error category, OS error code and numeric exit status only; command output and authentication material are excluded.

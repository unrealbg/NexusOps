# NexusOps

A native, agentless infrastructure control plane. The current milestone provides host management, verified SSH connections, read-only live Linux monitoring, independent interactive SSH PTY workspaces, and a safe SFTP file workspace with streamed transfers and a bounded remote text editor. Windows is the primary development target; the Tauri shell and Rust services support Windows, macOS and Linux.

No NexusOps software is installed on the remote machine. A working SSH server with an SFTP subsystem, a Linux user account and standard read-only utilities are sufficient. NexusOps requests no sudo, package installation, or service changes. Remote file writes, including text saves, occur only after an exact one-time file plan is shown and approved.

Services provides an on-demand, read-only snapshot of loaded systemd system services for a connected host. It supports manual Refresh and local filtering, with no service controls or background polling. See [services architecture](docs/architecture/services.md).

Network provides an on-demand, read-only snapshot of Linux interface names, operational states, MTUs and IPv4/IPv6 addresses for a connected host. It uses one fixed `ip -j address show` command, manual Refresh and local filtering, with no network controls or background polling. See [network architecture](docs/architecture/network.md).

Security displays local SSH endpoint pins and configured authentication metadata, including while offline. It reports verification only for a matching connected session. After a changed-key connection is blocked, **Review key change** presents the old pin and the key observed in that blocked handshake. Independently verify the new fingerprint before explicitly replacing the local endpoint pin; then reconnect normally. Planning and replacement do not read credentials, connect, authenticate or send remote commands. See [SSH endpoint trust architecture](docs/architecture/security.md).

Logs provides a bounded, read-only snapshot of up to ten accessible system journal entries from the current boot. It uses one fixed journalctl command with existing account permissions, no sudo or follow/polling, and manual Refresh. Potentially sensitive messages stay in component memory, are normalized for inert display and cannot be copied/exported through Logs. See [Logs architecture](docs/architecture/logs.md).

Containers provides an on-demand, read-only inventory of up to 64 recent containers in all states through the connected host's standard local Docker Engine system socket. It uses one fixed `docker --host unix:///var/run/docker.sock container ls` command with the SSH account's existing permissions, manual Refresh and no polling or container actions. Docker access can be highly privileged; NexusOps never grants it or changes permissions. Metadata remains in component memory. See [Containers architecture](docs/architecture/containers.md).

## Start

Install Node 24.15+, Rust 1.98.1 and your platform's [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then:

```sh
npm ci
npm run tauri -- dev
```

Create a host, select password or private-key authentication and save its credentials. Connect, compare the first-contact SHA-256 fingerprint with an independently verified value, and explicitly trust it. A changed key remains blocked; Security can guide an explicit local pin replacement after independent verification, followed by an ordinary fresh reconnect. Successful connection runs small read-only probes and populates Overview. While a connected Overview is visible, NexusOps samples CPU, memory, swap, root-disk and network counters every five seconds and keeps about ten minutes of history in memory. See [monitoring architecture](docs/architecture/monitoring.md). Hosts can be switched independently; disconnect before editing a connected host.

Select **Terminal** on a connected host to open one or more real `xterm-256color` SSH PTYs. Tabs, resizing, local scrollback search, explicit text clipboard actions, and full-screen terminal programs are supported. Terminal contents and tab metadata remain in memory only. See the [terminal architecture](docs/architecture/terminal.md) for ownership, limits, shortcuts, and sensitive-data handling.

Select **Files** to browse the account's resolved SFTP start directory, inspect metadata, upload/download ordinary files, edit bounded UTF-8 text files, and perform explicitly approved create, rename, or non-recursive delete actions. Local files and destinations are chosen through native dialogs; local handles and transfer payload bytes remain native-side. Bounded remote editor text and display-safe remote paths are explicit typed UI data. Transfers are staged, bounded, cancellable, and use explicit Skip, Keep both, or supported safe Replace behavior. See [SFTP files architecture](docs/architecture/sftp.md) and [remote text editor architecture](docs/architecture/editor.md) for the safety contracts and limitations.

`npm run dev` opens the frontend in a browser for UI work. It intentionally reports that desktop access is unavailable and does not simulate a connection. All sidebar sections are now implemented.

## Workspace

| Path | Responsibility |
| --- | --- |
| `apps/desktop` | React UI and thin Tauri command adapter |
| `crates/nexus-model` | Validated domain types and IPC models |
| `crates/nexus-core` | Host repository, provider composition, session orchestration |
| `crates/nexus-ssh` | SSH authentication, host-key verification, bounded sessions |
| `crates/nexus-terminal` | Typed PTY ownership, lifecycle, buffering and teardown |
| `crates/nexus-sftp` | SFTP v3 provider, typed file plans, paths and bounded transfers |
| `crates/nexus-secrets` | OS keychain abstraction and encrypted credential vault |
| `crates/nexus-discovery` | Independent probes, parsers and capabilities |
| `crates/nexus-operations` | Read-only command allowlist, planning and policy |
| `crates/nexus-audit` | Safe event schema, durable append and rotation |
| `packages/protocol` | TypeScript declarations generated from Rust |
| `packages/ui` | Design tokens, primitives and icons |

## Verify

```sh
npm run typecheck
npm run lint
npm test
npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p nexus-core --example export_protocol -- --check
npm run tauri -- build --no-bundle
```

The local SSH integration fixture binds loopback and generates its own keys. It does not require a VPS or system SSH daemon. The OS-keychain integration test is explicitly ignored by default because it needs an unlocked interactive keychain; see [setup and tests](docs/development/setup.md). Goal 01A's security baseline is in its [verification report](docs/validation/goal-01a-report.md). Goal 02A's terminal evidence is in the [terminal report](docs/validation/goal-02a-terminal-report.md).

The [release integrity foundation](docs/release/release-integrity.md) verifies version consistency and deterministic SHA-256 manifests for a native production executable. Product metadata is `0.1.1`, and the separate [production updater release runbook](docs/release/production-updater-release.md) defines exact signed-candidate, draft and activation boundaries. All release workflows are manual. No `0.1.1` candidate, tag, draft, GitHub Release or production `latest.json` has been created or published.

The [signed updater artifact foundation](docs/release/signed-updater-artifacts.md) prepares Tauri 2.12 version-bound signatures with an owner-held private key and a reviewable public key. Its separate candidate workflow is manual. The application offers a [manual update availability check](docs/architecture/updates.md) in the global topbar. A check can announce a newer version, a second action can download and verify it, and supported Windows builds can launch the exact verified NSIS artifact after a separate confirmation and full process quiescence. Linux and macOS installation, automatic restart and relaunch remain disabled. No GitHub Release or `latest.json` has been published yet.

Architecture, threat model, important limitations and architectural decisions are documented in [architecture](docs/architecture/overview.md), [monitoring](docs/architecture/monitoring.md), [terminal architecture](docs/architecture/terminal.md), [SFTP files](docs/architecture/sftp.md), [remote text editor](docs/architecture/editor.md), [security](docs/security/threat-model.md) and [ADRs](docs/adr/0001-workspace-and-boundaries.md). There is no AI execution, generic command IPC, sudo integration, remote agent, tunnel, bastion or alerting.

Licensed under MIT.

# NexusOps

A native, agentless infrastructure control plane. The current milestone provides host management, verified SSH connections, a read-only Linux overview, and independent interactive SSH PTY workspaces. Windows is the primary development target; the Tauri shell and Rust services support Windows, macOS and Linux.

No NexusOps software is installed on the remote machine. A working SSH server, a Linux user account and standard read-only utilities are sufficient. No sudo, package installation, remote file writes or service changes are performed.

## Start

Install Node 24.15+, Rust 1.98.1 and your platform's [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then:

```sh
npm ci
npm run tauri -- dev
```

Create a host, select password or private-key authentication and save its credentials. Connect, compare the first-contact SHA-256 fingerprint with an independently verified value, and explicitly trust it. A changed key is blocked. Successful connection runs small read-only probes and populates Overview. Hosts can be switched independently; disconnect before editing a connected host.

Select **Terminal** on a connected host to open one or more real `xterm-256color` SSH PTYs. Tabs, resizing, local scrollback search, explicit text clipboard actions, and full-screen terminal programs are supported. Terminal contents and tab metadata remain in memory only. See the [terminal architecture](docs/architecture/terminal.md) for ownership, limits, shortcuts, and sensitive-data handling.

`npm run dev` opens the frontend in a browser for UI work. It intentionally reports that desktop access is unavailable and does not simulate a connection. Services, Containers, Network, Security, Logs and Files are reserved, disabled navigation entries.

## Workspace

| Path | Responsibility |
| --- | --- |
| `apps/desktop` | React UI and thin Tauri command adapter |
| `crates/nexus-model` | Validated domain types and IPC models |
| `crates/nexus-core` | Host repository, provider composition, session orchestration |
| `crates/nexus-ssh` | SSH authentication, host-key verification, bounded sessions |
| `crates/nexus-terminal` | Typed PTY ownership, lifecycle, buffering and teardown |
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

Architecture, threat model, important limitations and architectural decisions are documented in [architecture](docs/architecture/overview.md), [terminal architecture](docs/architecture/terminal.md), [security](docs/security/threat-model.md) and [ADRs](docs/adr/0001-workspace-and-boundaries.md). There are no remote mutation features, AI execution, SFTP, tunnels, bastions or key-rotation UI.

Licensed under MIT.

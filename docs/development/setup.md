# Development setup

## Prerequisites

- Node 24.15 or later in the 24 LTS line; npm.
- Rust 1.98.1, selected by `rust-toolchain.toml` (rustfmt and Clippy included).
- Windows: Visual Studio Build Tools with Desktop development with C++, x64 MSVC libraries, Windows SDK, and the Edge WebView2 runtime. Use a Developer PowerShell if the compiler is not discovered automatically.
- macOS: Xcode Command Line Tools; a working user Keychain.
- Linux: WebKitGTK 4.1 and the [official Tauri prerequisite packages](https://v2.tauri.app/start/prerequisites/), plus an unlocked Secret Service implementation and session D-Bus for real credential use.

The frontend uses React 19, Vite 8, Query 5 and Zustand 5. TypeScript 6.0 is deliberately selected because the current typescript-eslint 8 release supports versions below 6.1; upgrading to TypeScript 7 without a compatible lint toolchain would violate mutual compatibility. Exact resolutions are committed in npm and Cargo lockfiles. Stable russh uses some prerelease cryptography dependencies internally; review the lockfile on updates.

## Run and build

```sh
npm ci
npm run tauri -- dev
# Compile a production executable without installers:
npm run tauri -- build --no-bundle
# Build native platform bundles, once signing/release settings are configured:
npm run tauri -- build
```

`npm run build` produces the frontend assets only. `cargo` commands that include the Tauri crate need those assets first. Browser-only development is `npm run dev`; it cannot access the desktop backend. No mock data is substituted.

## Tests and quality gates

```sh
npm run typecheck
npm run lint
npm test
npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --lib --bins --locked
cargo test -p nexus-ssh --test loopback --locked
cargo run -p nexus-core --example export_protocol -- --check
```

The loopback integration suite runs a real SSH protocol server in the test process, binds only `127.0.0.1` on an ephemeral port and uses generated keys and test credentials. Its command handler returns fixture data and never invokes a system shell. It covers unknown-before-auth verification, trusted reconnect, changed keys, password and encrypted private-key authentication, read-only discovery, cancellation/output bounds and independent hosts. It is separate from deterministic library tests and does not require an external server.

The following explicit integration test accesses the current user's OS keychain, creates a unique test key and encrypted temporary vault, then deletes the test key:

```sh
cargo test -p nexus-secrets --test platform_store -- --ignored
```

It is ignored in normal CI because headless runners may not have an unlocked keychain. Fake key providers are confined to tests; production always composes the OS-backed provider.

Goal 01A adds an independent OpenSSH fixture. It imports an owned Alpine WSL2 distribution, verifies the downloaded rootfs checksum, installs OpenSSH only in that disposable VM, runs the opt-in tests, and removes only the marker-owned distribution and scratch directory. See [OpenSSH interoperability validation](../validation/openssh-interoperability.md) for commands and safety details.

Goal 02A extends that disposable fixture with test-only terminal utilities and a real PTY test. From PowerShell, use a scratch path inside this task's `work` directory:

```powershell
$run = "<task-work>\goal-02a-openssh"
.\tools\openssh-fixture\Setup-OpenSshFixture.ps1 -RunRoot $run
.\tools\openssh-fixture\Start-OpenSshFixture.ps1 -RunRoot $run
.\tools\openssh-fixture\Run-OpenSshInterop.ps1 -RunRoot $run
.\tools\openssh-fixture\Cleanup-OpenSshFixture.ps1 -RunRoot $run
```

The combined run executes the existing authentication/trust phases plus `openssh_terminal_pty_interoperability`: shell output, UTF-8, ANSI styles, `stty size`, two independent PTYs, single-tab close, 2 MiB flow, `top`, and disconnect teardown. The fixture installs `vim` and terminal definitions only inside its owned Alpine VM. Its default distribution and marker are Goal 02A-specific; cleanup refuses an unmarked path.

Ed25519 is the supported client-key type for the Windows alpha. Optional RSA support is disabled because its current transitive Rust implementation has an unfixed timing-side-channel advisory. Do not enable legacy or obsolete SSH algorithms to work around server compatibility.

Regenerate the protocol after changing Rust DTOs:

```sh
cargo run -p nexus-core --example export_protocol
npm run typecheck
```

CI runs frontend checks, Rust formatting/Clippy/tests, protocol verification and a production Tauri executable build on Windows, macOS and Linux. The CI file's presence is not evidence that remote GitHub runs have passed; consult the workflow runs for a pushed repository.

## Data and troubleshooting

Tauri resolves the per-user app-data directory for `org.nexusops.desktop`. Typical locations are `%APPDATA%/org.nexusops.desktop` on Windows, `~/Library/Application Support/org.nexusops.desktop` on macOS and `$XDG_DATA_HOME/org.nexusops.desktop` (or `~/.local/share/...`) on Linux. Use the OS keychain unlock UI for `secureStorage` errors. Never work around them by saving credentials to config files.

`dns`, `refused`, `timeout` and `authentication` are separate UI failures. `unknownHostKey` requires comparing and trusting the observed key. `changedHostKey` requires independent investigation; it is deliberately not dismissible into a successful connection. For legitimate rotation, stop NexusOps, back up metadata, verify the new fingerprint independently and perform deliberate local pin maintenance. A guided audited rotation flow is deferred.

If an app-data profile is already open, close its other NexusOps process. Do not delete databases to solve the lock. Structured logs are in the profile's `logs` directory; audit contains action metadata only. Do not include credential vaults or OS keychain exports in issue reports.

Native verification may set `NEXUSOPS_TEST_APP_DATA_DIR` to an absolute disposable directory containing `.nexusops-isolated-test-profile` with exactly `nexusops-goal-01a-isolated` or `nexusops-goal-02a-isolated` and a trailing newline. The application refuses missing, relative, or unmarked test overrides. This hook is for owned test data only; routine development should use Tauri's normal per-user app-data path.

Terminal shortcuts and operational limits are documented in [terminal architecture](../architecture/terminal.md). The browser preview can render the workspace shell but cannot open a PTY because it has no native IPC or SSH transport.

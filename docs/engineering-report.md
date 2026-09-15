# Goal 01 engineering report

Date: 2026-09-14. Target verified locally: Windows x64.

NexusOps now has a working Tauri desktop application, a typed React host-management UI and a secure SSH-to-discovery vertical slice. It was implemented in a new repository because the supplied workspace contained no existing source. Remote interaction is agentless and read-only.

## Architecture implemented

The Cargo workspace contains seven domain/service crates and a thin desktop adapter. React components call a typed application client. The IPC command allowlist provides host CRUD, session status, connect/disconnect/reconnect, exact pending-key trust and discovery refresh. It contains no command execution, shell or remote filesystem API.

| Module | Implementation and important files |
| --- | --- |
| `nexus-model` | `host.rs`, `session.rs`, `error.rs`: UUID host IDs, validated configurations, authentication modes, lifecycle transitions, identity/capability/discovery DTOs and typed errors; operation risk model in `lib.rs` |
| `nexus-core` | `repository.rs`: SQLite metadata schema and credential references; `application/hosts.rs`, `connection.rs`, `identity.rs`, `lifecycle.rs`: orchestration, per-host generations, cancellation, trust and audit; `provider.rs`: transport creation interface |
| `nexus-ssh` | `provider.rs`, `handler.rs`, `known_hosts.rs`, `session.rs`, `bounded_io.rs`: SSH authentication, strict key verification, persistent pins, bounded execution and socket lifetime management |
| `nexus-secrets` | `lib.rs`, `vault.rs`: transient non-Debug credential input, secure-store interfaces, OS key provider and authenticated encrypted vault |
| `nexus-discovery` | `probes.rs`, `parsers.rs`, `capabilities.rs`: eight independent probes and validated capability registry |
| `nexus-operations` | Fixed read-only commands and transport interface; reviewable plans, risk validation, execution, output verification and explicit no-op rollback |
| `nexus-audit` | Typed actors/outcomes and metadata-only durable JSONL recording with bounded rotation |
| Desktop/UI | `src-tauri/src/commands.rs` and `main.rs`: composition and IPC; frontend components grouped by host and overview features; Query for remote state and Zustand for selection only |
| Shared packages | `packages/protocol/src/index.ts`: Rust-generated declarations; `packages/ui`: reusable accessible primitives, icons and theme tokens |

The required execution-flow diagram and extension boundaries are in [architecture/overview.md](architecture/overview.md). Future capabilities use the registry without changing the SSH transport. A future second transport should introduce a tagged connection configuration with explicit migration; speculative providers are not implemented.

## Major decisions

Four ADRs were written before and during incremental implementation:

1. [Workspace and boundaries](adr/0001-workspace-and-boundaries.md): domain/service crates, narrow IPC and generated DTOs.
2. [Host identity and secrets](adr/0002-host-identity-and-secrets.md): explicit TOFU, exact endpoint pins and no plaintext fallback.
3. [Read-only operations](adr/0003-read-only-operations-and-discovery.md): fixed commands, independent probes and fail-closed policy.
4. [Encrypted vault and revisions](adr/0004-encrypted-vault-and-credential-revisions.md): OS-stored master key with encrypted documents, immutable revisions and crash-safe metadata references.

Resolved stack versions include Rust 1.98.1, Tauri 2.11.5, Tokio 1.53.1, russh 0.63.3, keyring 4.2.0, AES-GCM 0.11.1, React 19.3.0, Vite 8.3.0, TanStack Query 5.102.8 and Zustand 5.0.15. TypeScript 6.0.3 is intentional: the selected current typescript-eslint version supports TypeScript below 6.1. Rust and npm lockfiles record the complete compatible dependency graph.

## Security design

The actual SSH server key is checked before authentication. Unknown keys close the handshake and produce hostname, algorithm and fingerprint for explicit review. Trust must match the current pending challenge, and SQLite serializes competing trust inserts. Existing pins cannot be overwritten through that API. Every subsequent connection verifies the key again; changed keys are blocked. Pins survive deletion of a host configuration.

Passwords, private keys and passphrases never enter plaintext metadata. AES-256-GCM protects credential documents, with random nonces and authenticated credential IDs. The master key is stored in Windows Credential Manager, macOS Keychain or Linux Secret Service. A persisted initialization marker prevents a missing master key from being silently regenerated, even after deleting all saved credentials. Rust secret buffers zeroize where supported; credential input has no Debug or public serialization implementation. React forms use transient uncontrolled secret inputs, clear them before awaiting submission, and bypass mutation caches.

Per-host state, transport and cancellation tokens are independent. Generation checks prevent obsolete connection/refresh tasks from publishing data after cancel, edit or disconnect. Keepalives and polling detect remote closure. DNS/handshake, authentication and probes have separate bounds. Combined command output is capped at 64 KiB; commands are a closed enum. Remote data is parsed as text and React renders it as text, never HTML.

Audit includes host, UTC timestamp, operation, actor, outcome and elapsed duration, with explicit cancelled outcomes. It never includes commands, remote output or secrets. Logs use safe stages/error codes instead of raw library/config dumps. CSP, local-window permissions and the absence of shell/filesystem plugins reduce the renderer's authority. See the complete [threat model](security/threat-model.md).

## Tests and quality results

All listed final quality gates passed on Windows. There are **69 passing tests** when the explicitly requested OS-store integration test is included:

| Suite | Passed | Evidence covered |
| --- | ---: | --- |
| Domain | 3 | Endpoint/name/username validation, IDs and lifecycle transitions |
| Core and repository | 9 | Persist/reopen/update/delete, multiple hosts, credential revisions and auth switching, cancellation isolation, stale-trust cleanup failure, remote-close refresh race, profile locking |
| Secrets | 8 | Encrypted roundtrip, no plaintext, tamper/AAD rejection, missing-master fail-closed behavior, legacy schema/ciphertext compatibility and maximum escaped credentials |
| SSH unit | 9 | Unknown/trusted/changed keys, endpoint and algorithm scoping, concurrent trust races, output bounds, deadlines and socket cancellation |
| SSH loopback integration | 8 | Actual handshake and authentication, encrypted private keys/passphrases, verification before credentials, changed real server keys, all probes, independent lifetimes and cancellation |
| Discovery | 8 | Parsing, optional failures, numeric bounds, old-kernel memory fallback and registry behavior |
| Operations | 5 | Risk ordering, rejection of every write risk, plan identity, verification, cancellation and deadlines |
| Audit | 4 | Closed schema, safe persistence, append/reopen, rotation and explicit failures |
| Windows secure-store integration | 1 | Real Credential Manager roundtrip and key-loss behavior with unique disposable test entries |
| Frontend | 14 | Forms, validation, credential lifetime/cache exclusion, trust UI, changed-key blocking, host switching and display formatting |

The OS-store integration test is ignored by the normal suite because a headless keychain may be unavailable. It was separately enabled and passed locally; it was not silently skipped. Normal Rust tests require no external VPS or system SSH daemon.

Commands executed for the final pass:

```sh
npm run typecheck
npm run lint
npm test
npm run build
npm audit --audit-level=high
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p nexus-secrets --test platform_store --locked -- --ignored
cargo run -p nexus-core --example export_protocol --locked -- --check
npm run tauri -- build --no-bundle
npm run tauri -- build
actionlint .github/workflows/ci.yml
```

The frontend build produces approximately 286.43 kB JavaScript (87.36 kB gzip) and 18.99 kB CSS. npm audit reported zero known vulnerabilities. Actionlint 1.7.12 returned no diagnostics; its official distribution checksum was verified. Rust Clippy passes with warnings treated as failures. Windows executable, MSI and NSIS builds succeeded.

## Native desktop verification

The production Windows executable was launched with WebView2, and its bundled page was inspected through a temporary loopback CDP debugging port. The test used actual UI controls and an isolated SSH fixture, with generated host keys and fixed Linux response data. It verified:

- Host creation through the form and encrypted credential persistence.
- Exact unknown-key display and explicit trust before successful connection.
- All eight discovery results and expected memory/disk percentages in Overview.
- Refresh, two independent host sessions, host switching without stale metrics, disconnect, editing and saved-credential reuse.
- An arbitrary-command IPC invocation is rejected.
- Restarting the fixture on the same port with a different key blocks reconnection, displays both fingerprints and offers no trust override.
- Deleting both test hosts removes their saved credentials. Test database/log scans found no test password in plaintext.

The test hosts, their credentials and generated endpoint pin were removed afterwards. Test processes and the preview server were stopped. The temporary debugging port is not configured in production code. Raw native captures and browser fixture captures remain local validation evidence and are excluded from source publication because they can contain workstation details. Their results are summarized in the Goal 01A and Goal 02A validation reports.

## CI and documentation

[GitHub Actions](../.github/workflows/ci.yml) runs frontend typecheck/lint/tests/build, Rust formatting/Clippy/unit tests, separate SSH integration, protocol drift checks and production executable builds across Windows, macOS and Ubuntu. It uses read-only repository permissions and fails on meaningful gate failures. The YAML is validated; no GitHub remote was supplied or workflow dispatched, so remote CI results are not claimed.

The repository includes README, setup instructions, architecture diagram, threat model, four ADRs, SSH integration instructions, frontend architecture/visual QA notes and an MIT license.

## Resolved implementation and environment issues

The existing npm cache drive was full, the default Node runtime was too old for current Vite, and the Visual Studio installation lacked required SDK/libraries. Work used a task-local npm cache, the bundled Node 24.19 runtime and verified official Microsoft compiler/SDK packages extracted into task scratch. Large build caches live on a scratch junction on a drive with capacity. No remote host was touched to address local build prerequisites.

Review found and fixed stale Connected publication after pin-read failure, stale trust after partial credential cleanup, missing-master reinitialization, 64-bit TypeScript metric mappings, escaped-credential size bounds, cancellation auditing and npm flag forwarding. Initial tool/harness failures were resolved before the final pass; no failing lint, test or build remains.

## Compromises, limitations and known risks

- Native execution and real OS-store verification were performed on Windows. macOS/Linux have platform implementations and CI configuration, but were not executed in this environment.
- The SSH fixture is a real protocol peer with fixed Linux responses. Independent OpenSSH/server-distribution interoperability is a release-readiness check; no external VPS was supplied or used.
- Installers are unsigned. Code signing, notarization, update distribution and published releases are not configured.
- TOFU requires independent first-contact fingerprint verification. A logged-in account compromise can access active secrets or alter local metadata/pins; memory/IPC buffers cannot be universally wiped.
- No key-rotation UI, SSH certificates, agent authentication/forwarding, jump hosts, tunnels or terminal is implemented. Private keys are pasted as PEM; file import is deferred.
- Started OS keychain calls and private-key KDF work cannot be forcibly interrupted. Cancellation invalidates the result and tears down transport; blocking work may finish afterwards.
- Crashes can leave encrypted orphan credential revisions. Audit is durable and bounded but not tamper-evident. An audit/cleanup error can be reported after a local action has committed; refresh reveals the committed state.
- Linux discovery depends on standard utilities and procfs; unavailable metrics remain explicit warnings. Only load, not sampled CPU percentage, is collected. Light-mode architecture is prepared, but only dark mode ships.
- No independent security audit or comprehensive Rust dependency advisory scan is claimed. Stable russh currently resolves some prerelease cryptographic crates; dependency review remains part of release work.

## Suggested Goal 02

Add read-only systemd service inventory and journal views behind a real capability provider. Exercise typed paginated/streaming responses, cancellation and bounded log redaction without introducing remote writes. In parallel, add an OpenSSH distro integration matrix and signed desktop release pipeline before broader distribution.

## Goal 01A correction and verification addendum

The original report's 69-test total consists of 54 default Rust tests, one separately enabled Windows Credential Manager integration test, and 14 frontend tests. Native and manual observations were never included in that count. Goal 01A added one core shutdown regression, so the final default Rust count is 55; the Windows keychain test and two real OpenSSH phases remain separately invoked environment tests.

Goal 01A replaced the prior OpenSSH limitation with actual interoperability evidence against OpenSSH 10.3p1 on a disposable Alpine 3.24 WSL2 VM. Password, Ed25519, encrypted Ed25519, trust rejection/acceptance, persistent trust, real discovery, independent sessions, reconnect, timeout, and same-endpoint key rotation passed. A fresh native Windows release run also proved Windows Credential Manager persistence across restart and the key-A-displayed/key-B-subsequent race. See [Goal 01A report](validation/goal-01a-report.md) and [OpenSSH interoperability](validation/openssh-interoperability.md).

Two hardening changes resulted: shutdown now clears stale session views and invalidates attempt generations, and optional RSA support was disabled after RustSec identified an unfixed timing-side-channel advisory. Ed25519 is the supported Windows-alpha client-key type. The final `cargo-audit` run reports no vulnerabilities; its remaining warnings are transitive maintenance items outside the evaluated Windows binary.

## Goal 02A terminal addendum

Goal 02A adds a separate `nexus-terminal` subsystem and seven typed Tauri PTY commands. Terminals are owned by host, authenticated connection identity and terminal ID; reconnect cannot attach an old PTY. The SSH layer requests a persistent `xterm-256color` PTY and the user's normal shell, while bounded byte queues feed xterm 6 without persisting input, output, labels or scrollback. Multi-tab and multi-host workspaces, text-only clipboard actions, local search, resize, explicit ended states and application shutdown teardown are implemented.

The final deterministic suites contain 62 default Rust tests and 18 frontend tests. Separately enabled Windows Credential Manager and three real OpenSSH tests passed. The actual Windows release passed sanitized native checks for ANSI, UTF-8, copy/paste, search, resize, two PTYs, two hosts, `top`, Vim, 12,000 lines, close, disconnect/reconnect, stale-ID rejection and process restart. See the [terminal engineering report](validation/goal-02a-terminal-report.md) and [terminal architecture](architecture/terminal.md).

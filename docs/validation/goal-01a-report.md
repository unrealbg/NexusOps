# Goal 01A engineering verification report

Date: 2026-09-14  
Scope: NexusOps Goal 01 Windows SSH/discovery alpha  
Assessment type: engineering verification and focused hardening performed alongside implementation; this is not an independent third-party security audit.

## Acceptance decision

The original Windows vertical slice is demonstrated against an independent OpenSSH implementation. The host-key, credential, Tauri boundary, lifecycle, discovery, native release, and required local quality checks pass. No critical or high-severity finding remains in the evaluated scope. The decision is limited to the Windows alpha described here; macOS and Linux application runtime behavior, code signing, installer reputation, and hosted CI execution remain unverified.

## Baseline

The repository has Git metadata but no commit: `git rev-parse HEAD` returned no revision, and `git status --short --branch` reported `No commits yet on master` with the source tree untracked. No existing change was discarded. Before Goal 01A changes, a deterministic hash over 131 sorted source paths and their SHA-256 values was `12f1d8983c3a744c83cdfd927679bb63af9704bc12c746762ae084fc3594d621`.

Host environment:

| Item | Observed value |
|---|---|
| OS | Windows 11 Pro 64-bit, 10.0.26200 build 26200, x64 |
| Rust | `rustc 1.98.1 (48a229cea 2026-09-01)`, `x86_64-pc-windows-msvc`, LLVM 22.1.8 |
| Cargo | 1.98.1 |
| Project Node | 24.19.0 from the bundled Codex runtime |
| npm | 10.8.2 |
| Default system Node | 22.7.0; not used because Vite 8 requires a newer runtime |
| Tauri | JS 2.11.1, CLI 2.11.4, Rust 2.11.5 |
| React / Vite / TypeScript | 19.3.0 / 8.3.0 / 6.0.3 |
| TanStack Query / Zustand | 5.102.8 / 5.0.15 |
| Tokio / russh | 1.53.1 / 0.63.3 |
| keyring / AES-GCM / rusqlite | 4.2.0 / 0.11.1 / 0.40.2 |

The original baseline gates all passed before substantial changes: Rust formatting, Clippy with warnings denied, 54 default Rust tests, the one ignored Windows credential-store test, protocol generation check, TypeScript typecheck, ESLint, 14 Vitest tests, frontend production build, and npm audit. The earlier “69 passing tests” claim is therefore accurate only when described as 54 default Rust tests + 1 separately invoked Windows keychain integration test + 14 frontend tests. Native/manual observations were not part of that count.

## Requirement-to-evidence matrix for Goal 01

| Original requirement | Result | Evidence |
|---|---:|---|
| Tauri 2/Rust/Tokio desktop and React/TypeScript/Vite frontend | PASS | Manifests, locks, local builds, and native release execution. |
| Multi-crate architecture and typed SSH-independent domain | PASS | Separate model, core, SSH, secrets, discovery, operations, audit, desktop, UI, and protocol packages; no `russh` types cross the domain boundary. |
| Host CRUD and multiple independent profiles | PASS | Core CRUD regressions plus native creation and switching of two hosts. |
| Password and private-key authentication | PASS | Real OpenSSH password and Ed25519 key tests, including encrypted key/passphrase. |
| Connect/disconnect/reconnect/state/errors/cancellation/timeouts | PASS | Loopback tests, real OpenSSH phases, core lifecycle tests, and native reconnect. |
| Unknown/trusted/changed host-key behavior | PASS | Backend unit/integration tests, real OpenSSH key rotation, and native A-to-B stale-approval scenario. |
| Credentials outside plaintext/browser/log storage | PASS | Windows Credential Manager integration, AES-GCM vault inspection, canary scans, browser storage assertion, and safe logging review. |
| Read-only bounded discovery and partial failure | PASS | Eight separate probes, parser tests, command/output/time bounds, loopback partial-failure test, and real OpenSSH discovery. |
| Initial capability registry | PASS | Typed `HostCapability` output and `linux`/`procfs` detection. |
| Operation/policy architecture and risk levels | PASS | Fixed `ReadOnlyCommand` enum and `OperationRisk`; no generic remote command API. |
| Structured audit without secrets/output | PASS | JSONL audit model and canary/source review. |
| Professional desktop UI and host overview | PASS | Fresh native screenshots and behavior checks. |
| Typed safe error model | PASS | DNS, timeout, refused, authentication, trust, discovery, secure-storage, cancellation, connection, conflict, and policy codes. |
| CI definition | PASS | Workflow exists, uses locked Rust commands and frontend gates; actionlint 1.7.12 passed locally. |
| Hosted CI run | NOT VERIFIED | No remote was pushed and no hosted run was available. |
| Documentation and ADRs | PASS | README, architecture, threat model, setup, four ADRs, and Goal 01A validation documents. |
| Zero NexusOps software on managed hosts | PASS | Real discovery required only stock OpenSSH; OpenSSH installation occurred solely in the disposable test VM. |
| Windows runtime | PASS | Actual release executable and real OS credential backend were exercised. |
| macOS/Linux app runtime | NOT VERIFIED | Cross-platform source/lock inspection is not runtime evidence. |

## Findings and fixes

### Medium: optional RSA implementation carried RUSTSEC-2023-0071

Source: `crates/nexus-ssh/Cargo.toml` and `Cargo.lock`.

`crates/nexus-ssh/Cargo.toml` enabled `russh`'s optional RSA feature. `cargo-audit 0.22.2` reported the Marvin timing-side-channel advisory in `rsa 0.10.0-rc.18`; upstream offered no fixed version. The Windows alpha has independent runtime evidence for Ed25519 and no requirement for RSA. The optional feature was removed, the lockfile was regenerated by Cargo, and the final audit reports zero vulnerabilities. Ed25519 is now the explicitly supported alpha client-key type.

### Medium: shutdown retained stale connected/connecting view state

Source: `crates/nexus-core/src/application.rs` and regression coverage in `crates/nexus-core/src/application/tests.rs`.

`Application::shutdown` cancelled tokens and removed transports but left `HostSession.state` unchanged. A caller observing the object during shutdown could see a connected or connecting state after transport teardown. Shutdown now increments the attempt generation, cancels work, clears refresh and transport state, and replaces the view with `Disconnected`. `repeated_connect_and_shutdown_leave_no_live_or_stale_session` covers duplicate connect rejection, an uncooperative late result, shutdown during connect, and shutdown after a successful connection.

### Low: environment-only Windows data redirection was ineffective

Source: `apps/desktop/src-tauri/src/main.rs`.

Windows known-folder resolution does not reliably follow a process-local `APPDATA` override. The desktop bootstrap now supports `NEXUSOPS_TEST_APP_DATA_DIR` only when it is absolute and contains `.nexusops-isolated-test-profile` with the exact ownership marker. This allowed native release verification without deleting or replacing the owner's profile. Invalid or unmarked overrides fail application setup. The hook does not expose data through IPC and requires control of the local process environment and marker directory.

Before this hook existed, one temporary native-test host was inadvertently created in the default profile because the attempted process-local `APPDATA` redirect was ineffective. That host and its credential revision were deleted through the application, then the exact endpoint pin and the nine audit events bearing that temporary host ID were removed. No unrelated credential, host, pin, or audit row was enumerated or removed.

### Informational: Docker fixture unavailable

Docker Desktop reported `preparing block device /dev/sde: mounting disk: input/output error`. It was not restarted or modified because that could disturb unrelated Docker state. The test used a newly imported, marker-owned WSL2 Alpine VM and left the existing `Ubuntu` distribution untouched.

### Informational: native control plugin unavailable

Both installed computer-control entry points failed before initialization with `failed to write kernel assets: The system cannot find the path specified`. Native verification therefore used WebView2 DevTools automation attached to the actual release Tauri process. This is native-process/backend evidence, not a browser preview: every host, trust, credential, SSH, discovery, and delete action crossed Tauri IPC into the release Rust backend. Screenshots came from that execution. The unavailable control plugin is a tooling limitation, not an application defect.

## Host-key trust audit

The call path is `HostOverview` → fixed `hostApi` command → Tauri `trust_host_key`/`connect_host` → `Application` → `SshProvider` → `russh` handshake handler → `KnownHosts`.

- `russh::client::connect_stream` invokes the handler's server-key check during handshake. `SshProvider` begins password/public-key authentication only after `connect_stream` succeeds.
- Unknown keys return a backend-generated `HostKeyChallenge`; authentication and discovery do not run. OpenSSH log assertions confirm no accepted auth or session start.
- `Application::trust_host_key` accepts only the exact challenge currently owned by a session in `AwaitingTrust`, rejects challenges with a prior fingerprint, and rejects stale/edited/different endpoint, port, algorithm, or fingerprint data.
- `KnownHosts::trust` uses an immediate SQLite transaction and insert-if-absent semantics. A concurrent or changed key cannot overwrite the stored value.
- DNS names are trimmed, trailing dots removed, and lowercased. IP literals use canonical `IpAddr` formatting. Trust is keyed by canonical hostname/IP plus port; algorithm and fingerprint are stored values. Duplicate host profiles for the same canonical endpoint and port intentionally share a pin. Different ports have independent pins.
- The native A-to-B test showed that approval of displayed A only pinned A; B was blocked on the subsequent handshake and could not be accepted from the changed-key UI.

On Windows, the trusted-host SQLite files inherit the signed-in user's app-data ACL. The production profile observed full control for the user, SYSTEM, Administrators, and its app-container SID, plus read/execute for the task environment's `CodexSandboxUsers` group. SQLite transactions provide atomic row changes. On Unix, `Application::open` explicitly sets the profile directory to mode 0700. No “connect anyway” path exists.

## Credential and secret handling

The production Windows backend is keyring 4.2.0's Windows Credential Manager store. It holds a random 32-byte vault master key under a per-profile account. The application stores only AES-256-GCM ciphertext, a random nonce, and immutable credential revision ID in SQLite; the revision ID is authenticated as associated data. A persistent vault marker makes a missing master key fail closed.

The ignored Windows integration test now covers password and private-key/passphrase documents, separate host associations, application-style immutable updates, vault reopen, deletion, encrypted-file canaries, and missing-key failure. The native run confirmed a saved password survived a process restart and authenticated again without renderer input. After the run, both hosts and both credential revisions were deleted through the app, the isolated file scan found no plaintext password or key passphrase, and the exact test profile's Credential Manager master key was deleted without enumerating the user's store.

A final exact-canary scan checked 147 source, evidence, executable, MSI, and NSIS files for the generated password, passphrase, and complete private-key payloads and found no matches. A focused common-token scan found only the intentional `BEGIN OPENSSH PRIVATE KEY` input placeholder; it found no embedded private key or common AWS/GitHub token pattern. These focused scans supplement the code and runtime review; they are not a claim that every possible secret pattern is absent.

General frontend APIs return `Host` and `HostSession`, never credentials. `CredentialInput` is deserialize-only and has no `Debug` or `Serialize`; Rust secret buffers use `Zeroizing` where supported. React uses uncontrolled secret controls, bypasses TanStack mutation caches, and clears values before awaiting submission and on cancellation/failure dismissal. JavaScript strings and all third-party/OS buffers cannot be guaranteed erased. Private key material is pasted transiently, encrypted into the vault, and is not copied to a plaintext application file; local key paths are not persisted.

Implemented and runtime-tested: Windows Credential Manager. Implemented by keyring but not runtime-tested here: macOS Keychain and Linux Secret Service. Session-only storage is not implemented and no plaintext fallback exists.

## Tauri/API boundary

No Tauri plugins are registered. The `main` capability is restricted to the bundled `main` window and exactly nine generated command permissions. Remote web content is not configured; production navigation uses bundled assets. Production CSP limits scripts to self, styles to self/inline style declarations, images to self/asset/data, and connections to Tauri IPC. Object, base, frame, and form targets are disabled; prototype freezing is enabled. React renders server values as text and no `dangerouslySetInnerHTML` use exists.

| Command | Purpose and validation/policy | Secret/file/remote effect |
|---|---|---|
| `list_hosts` | Lists validated repository records. | Metadata read only; no secrets or remote work. |
| `save_host` | Validates name, hostname/IP, port, username, auth method; blocks active-session edits; writes an immutable credential revision before metadata commit. | May write encrypted credential data and OS vault key; no local arbitrary path or remote work. |
| `delete_host` | Requires an existing host; disconnects first; deletes metadata then owned credential revision. | Removes only app-owned host/credential data; no remote write. |
| `connect_host` | Requires saved host and credential; serializes state transition; verifies host key, authenticates, then runs fixed discovery. | Reads one credential; fixed read-only remote probes only. |
| `disconnect_host` | Requires existing host; increments generation and cancels host lifetime. | Closes one owned session. |
| `reconnect_host` | Runs typed disconnect then connect. | Same bounded effects as those commands. |
| `get_session` | Requires existing host; detects a closed transport and clears stale discovery. | Returns typed state/discovery; no secrets. |
| `trust_host_key` | Requires exact backend-owned pending challenge; changed/stale challenges fail. | Atomically adds one first-use pin; no credential read or remote command. |
| `refresh_host` | Requires one connected, non-refreshing session and current generation. | Executes only the eight fixed read-only probes. |

The release-process behavioral test attempted `execute_command` with `uname -a`; Tauri rejected the unregistered command. There is no generic command or local-file IPC API. A source review was used to enumerate the boundary, and the native invocation supplied the behavioral check.

## Session and discovery verification

Automated coverage includes cancellation before connection, cancellation during a stalled connection, cancellation during discovery, awaiting-trust cancellation, duplicate connect rejection, late-result generation checks, independent host sessions, remote closure/network loss detection, switching hosts, active-host edit/delete policy, refresh races, disconnect/reconnect, and shutdown. Stale attempts cannot publish because every mutation increments a per-host generation and cancels the prior token before the later result acquires state.

TanStack query keys include the host ID, session polling is per host, and selection state contains only the selected ID. The native two-host check confirmed a disconnected host did not display the other host's discovery result.

Discovery uses eight separate fixed enum commands; no user or remote text is interpolated into a shell command. Each command has a 10-second limit, channel close has a 2-second limit, combined stdout/stderr is capped at 64 KiB, nonzero exit and SSH failure are typed discovery failures, and optional probe failures become warnings while preserving the SSH connection. Remote text is parsed as data. Missing values remain `None` and display as unavailable. `load_one` is explicitly shown as a one-minute system load average; it is not labeled CPU percentage. Bytes are stored as bytes and formatted into binary units in the UI. No fallback metrics are fabricated.

## Native Windows release evidence

The exact final release binary was launched from `work/target/release/nexus-desktop.exe` with a marker-owned isolated data directory. The test created two hosts, rejected then accepted an unknown key, authenticated through Windows Credential Manager, displayed live OpenSSH discovery, rejected arbitrary IPC, restarted the process and reconnected with persisted trust/credentials, switched host views, and blocked the A-to-B key change. All generated test hosts, credential revisions, app data, VM data, and the exact Credential Manager test key were removed.

The machine-readable result summary is published as `docs/validation/goal-01a-native-results.json`. The UI was visually inspected after capture. It displayed endpoint, algorithm, SHA-256 fingerprint, Alpine/WSL discovery data, units, connected state, and the changed-key block without secrets. Raw native captures remain local validation evidence and are excluded from source publication because they can contain workstation details.

Final unsigned development/alpha artifacts:

| Artifact | SHA-256 |
|---|---|
| `nexus-desktop.exe` | `8f258bd72d59e2f8a2c8b1f821577f4801c8d21e520eba5c219d828f0851519b` |
| `NexusOps_0.1.0_x64_en-US.msi` | `653184c6646fc165d3572a7c3ce3852d3cb5ffd4252d006fcd629c3c770409d4` |
| `NexusOps_0.1.0_x64-setup.exe` | `ef2f1861bb68a8f7563a74bde03c71afaa1f6158667100fc04ef488fd60a7879` |

The exact 108 build-input files under Cargo manifests/lock, package manifests/lock, `apps/`, `crates/`, and `packages/` are recorded in `docs/validation/goal-01a-build-inputs.sha256`, sorted by repository-relative path. The manifest itself has SHA-256 `a0796a7a53567dba9b93e67f512deb987847395131c592ead0f6992dbbf9fbd0`. There is no Git commit to cite. Documentation, evidence files, generated frontend output, dependencies, and compiler output are excluded from that source manifest.

## Final validation commands and results

Commands use the task-scoped MSVC/Windows SDK and bundled Node 24 environment described in `docs/development/setup.md`.

| Command | Result |
|---|---:|
| `cargo fmt --all --check` | PASS; exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS; exit 0 |
| `cargo test --workspace --locked` | PASS; exit 0; 55 default Rust tests, 3 ignored environment tests |
| `cargo test -p nexus-secrets --test platform_store --locked -- --ignored --exact platform_keychain_persistence_update_association_and_delete` | PASS; exit 0; 1 Windows keychain integration test |
| `tools/openssh-fixture/Run-OpenSshInterop.ps1` | PASS; exit 0; 2 real OpenSSH phases, invoked separately |
| `cargo run -p nexus-core --example export_protocol --locked -- --check` | PASS; exit 0 |
| `npm run typecheck` | PASS; exit 0 |
| `npm run lint` | PASS; exit 0 |
| `npm test` | PASS; exit 0; 14 frontend tests in 5 files |
| `npm run build` | PASS; exit 0; 0.52 KiB HTML, 18.99 KiB CSS, 286.43 KiB JS before gzip |
| `npm run tauri -- build` | PASS; exit 0; release executable, MSI, and NSIS bundles |
| `npm audit --audit-level=high` | PASS; exit 0; 0 vulnerabilities |
| `cargo audit --file Cargo.lock` (cargo-audit 0.22.2) | PASS; exit 0; 0 vulnerabilities, 7 warnings |
| `actionlint` (1.7.12) | PASS; exit 0; no workflow diagnostics |
| Native release phases | PASS; 3 phases and 13 recorded checks |

Rust test breakdown is separate from other evidence:

| Category | Count | Notes |
|---|---:|---|
| Rust unit tests in default suite | 47 | Audit 4, core 10, discovery 8, model 3, operations 5, secrets 8, SSH 9. |
| Rust integration tests in default suite | 8 | Deterministic in-process SSH protocol fixture. |
| Rust environment integration tests | 3 | One Windows keychain test and two real OpenSSH phases, invoked explicitly because they are ignored by default. |
| Frontend unit/component tests | 14 | Five Vitest files. |
| Native end-to-end phases | 3 | External WebView2 automation of the actual Tauri release; not included in unit-test totals. |
| Manual inspections | uncounted | Source/API review, screenshot review, ACL and cleanup checks, CI/lockfile review. |

The seven RustSec warnings are six unmaintained transitive crates and one `glib 0.18.5` iterator-unsoundness warning from Tauri's non-Windows GTK/WebKit dependency graph. They are not vulnerabilities in the evaluated Windows binary. They remain dependency-maintenance items for future Linux work and do not justify an indiscriminate framework upgrade in this milestone.

CI status is intentionally separated: the workflow was inspected, actionlint validated its syntax/semantics locally, and equivalent commands passed locally. Hosted CI was not run because no push or external publication was authorized.

## Remaining limitations

- Windows alpha artifacts are unsigned development builds and were not installed through every upgrade/uninstall path.
- macOS Keychain, Linux Secret Service, macOS/Linux Tauri runtime, IPv6 SSH, restrictive remote accounts missing individual probe tools, and network loss during a real OpenSSH discovery were not runtime-tested here. Deterministic regressions cover partial probe failure and network/session closure semantics.
- First-use trust remains TOFU and requires the user to compare the fingerprint through a separate trusted channel.
- A compromised logged-in Windows account can read process memory, call user-level APIs, or alter app-owned data. JavaScript and third-party allocator buffers cannot be guaranteed erased.
- RSA client keys are unsupported in this Windows alpha until a fixed upstream implementation and dedicated interoperability coverage are available.
- Hosted CI status is unknown.

These limitations do not block Goal 02A within the scoped Windows alpha. Terminal work must retain the existing typed policy boundary and must not turn the current renderer API into a generic unrestricted command surface.

**READY FOR GOAL 02A — TERMINAL**

# Goal 02A terminal engineering report

Date: 2026-09-14  
Platform: Windows 11 x86-64, native Tauri/WebView2 release  
Scope: typed SSH PTY subsystem, multi-terminal workspace, OpenSSH integration, focused security review, and regression verification  
Assessment: implementation and engineering verification; not an independent security audit

## Result

NexusOps now opens genuine persistent SSH PTY channels from a dedicated typed terminal API. A user can create independent terminal tabs, run the remote account's normal shell and full-screen programs, resize the remote PTY, search local scrollback, copy and paste through explicit text-only actions, switch among isolated hosts, close individual channels, and reconnect without reusing stale PTYs.

Goal 01/01A host-key and credential boundaries remain in force. The renderer did not gain generic command execution, filesystem access, or an SSH library. Terminal input and output are not stored or logged by NexusOps. No Critical or High severity issue remains in the evaluated Goal 02A scope.

## Architecture implemented

The new `nexus-terminal` crate owns session identity, state, queues, cancellation, and teardown. Each entry is keyed and checked with `(HostId, HostSessionId, TerminalSessionId)`. Core issues a new connection ID after every successful SSH authentication and rejects a terminal operation unless that exact connection is still active. The frontend cannot access a terminal with a terminal ID alone.

`nexus-ssh` implements the terminal connector on the authenticated `SshSession`. It opens a real SSH session channel, requests an `xterm-256color` PTY with character and pixel dimensions, waits for server acknowledgement, requests the account's normal shell, and preserves output that arrives during startup. The channel remains open for interactive byte streaming; it does not repeatedly execute commands.

Tauri exposes seven specific commands: list, open, poll, write, resize, rename, and close. Generated capabilities grant those commands only to the bundled main window. The operation engine remains reserved for fixed structured operations.

The React workspace uses `@xterm/xterm` 6.0.0, `@xterm/addon-fit` 0.11.0, and `@xterm/addon-search` 0.16.0. Each host owns mounted xterm instances for the current process, while each tab maps to one backend PTY. The frontend keeps terminal bytes out of React, Query, and Zustand state. [Terminal architecture](../architecture/terminal.md) contains the full flow and lifecycle diagram.

## PTY lifecycle and disconnect semantics

States are `creating`, `open`, `closing`, `closed`, `failed`, and `disconnected`. Startup is bounded to ten seconds, stream and resize operations to ten seconds, and close to two seconds. Closing stops new input, cancels the pump, sends EOF/close, drops queued output, releases the channel, and records an idempotent bounded tombstone. A remote exit or channel close releases the stored channel promptly.

Host disconnect marks terminals from that connection as disconnected. Reconnect creates another `HostSessionId`; ended tabs show a Connection lost overlay and can only open a new terminal. Deleting a host closes and removes all its entries. Application shutdown closes all channels and clears terminal memory. Native restart validation confirmed no session or scrollback reappeared as live.

## Binary streams, resize, and backpressure

Input/output remains bytes through xterm, typed IPC, the Rust manager, russh, and the PTY. Base64 is only the IPC envelope. xterm receives `Uint8Array`, preserving incremental UTF-8 and ANSI parsing across packet boundaries. Input calls are non-empty and limited to 16 KiB; the UI briefly batches keystrokes and splits larger paste input.

Each terminal has a 64-slot queue of chunks no larger than 16 KiB, about 1 MiB maximum queued output. The producer awaits capacity, allowing backpressure to reach russh's 64 KiB SSH receive window. Each poll returns at most 64 KiB. xterm scrollback is capped at 10,000 lines. A host can have at most eight creating/open terminals; ended records do not consume the limit.

FitAddon sizes the visible emulator. ResizeObserver changes are coalesced over 80 ms and carry columns, rows, device-pixel width, and device-pixel height. The real OpenSSH test verified a 101 by 37 character request as `37 101` from `stty size`; native viewport resizing changed the backend session dimensions.

## Clipboard, search, and keyboard policy

The Tauri clipboard manager is restricted to `allow-read-text` and `allow-write-text`; image, HTML, clear, and broad/default clipboard permissions are absent. Clipboard reads and writes occur only after Copy or Paste. Native validation selected a local xterm search result, copied it with `Ctrl+Shift+C`, read the resulting system text, seeded a sanitized command, and pasted it with `Ctrl+Shift+V`.

SearchAddon performs next/previous search inside the local xterm scrollback. Search terms do not cross IPC. The UI also provides Search and Clear controls plus a restrained Copy, Paste, Select all, Clear, Search context menu.

Application shortcuts are `Ctrl+Shift+C` Copy, `Ctrl+Shift+V` Paste, `Ctrl+Shift+F` Search, `Ctrl+Shift+K` Clear, `Ctrl+Shift+T` New terminal, and `Ctrl+Shift+W` Close. Other keys are returned to xterm, including `Ctrl+C/D/Z/L/R`, Tab and Shift+Tab, arrows, Home/End, Page Up/Down, Escape, function keys, and Alt combinations. Tab controls are keyboard accessible and support Left/Right navigation.

## Automated verification

Automated counts include test-runner tests only. They exclude compile checks, audits, builds, OpenSSH environment tests, and native interaction observations.

| Suite | Result | Count and coverage |
| --- | --- | --- |
| Rust default workspace | PASS | 62 passed, 0 failed, 4 environment tests ignored. This includes 5 terminal-manager tests, 11 core tests, 9 SSH unit tests, and 8 real-protocol loopback tests. |
| Frontend Vitest | PASS | 18 passed across 6 files. Four terminal tests cover tabs, mapping and host isolation, disconnected/search behavior, and connect gating. |
| Rust formatting | PASS | `cargo fmt --all -- --check` returned 0. |
| Rust compile | PASS | `cargo check --workspace --all-targets --locked` returned 0. |
| Rust lint | PASS | strict workspace/all-target Clippy with `-D warnings` returned 0. |
| TypeScript and ESLint | PASS | Typecheck and lint returned 0. |
| Generated protocol | PASS | Re-export produced the same SHA-256 and TypeScript then passed. |
| Frontend production build | PASS | Vite transformed 89 modules; a 664.91 kB main chunk generated with a size advisory only. |

The terminal-manager tests cover lifecycle, byte input and resize, idempotent close, host/connection ownership, creation cancellation, reconnect non-reuse, binary/UTF-8 boundary handling, late output, bounded queue behavior, and bounded poll batches. The core regression proves old connection identities cannot be reused. Existing credential, trust, discovery, operation, audit, cancellation, and loopback SSH regressions all remained green.

Vitest prints jsdom's expected canvas-not-implemented diagnostic while xterm is mocked; there is no failed assertion. The actual WebView2 canvas path was exercised by native validation.

## Integration verification

### Real OpenSSH

The disposable fixture was Alpine 3.24.0 with `openssh-server-10.3_p1-r1` on loopback port 20196. Goal 02A installed `ncurses-terminfo-base` and Vim inside that fixture only. The combined opt-in run passed three separate tests:

1. phase A: first-contact rejection, trust, password and Ed25519 authentication, encrypted Ed25519, persistence, real discovery, concurrency, reconnect, failure categories, and timeout;
2. terminal: real PTY/shell output, Bulgarian Cyrillic and UTF-8 symbols, ANSI data, `stty size`, two concurrent PTYs, stream isolation, closing one while the other survived, 2 MiB `yes | head` flow, real `top`, and disconnect teardown;
3. phase B: same-endpoint host-key rotation was rejected before authentication.

The PTY terminal type was `xterm-256color`. Tested rendering behavior included red ANSI color, reset, bold, underline, reverse, alternate-screen redraw in `top`, and Vim interaction in the native app. The emulator supports 256-color parsing naturally. `COLORTERM` and a remote TrueColor environment were not asserted.

The separately enabled Windows Credential Manager integration test passed once and deleted its unique test key. The default Rust count does not include that test or the three OpenSSH tests.

### Native Windows release

The final release executable was built from the recorded source state, launched with a marker-owned isolated profile, and controlled through its real WebView2 surface. It connected to the disposable OpenSSH server and completed these sanitized checks:

- first-contact trust and real Windows credential storage;
- PTY shell input/output and ANSI rendering;
- viewport-to-backend PTY resize;
- local search, explicit native copy, and explicit native paste;
- two independent tabs, rename, rapid switch, single-tab close, and sibling survival;
- `top` redraw/exit and Vim alternate-screen edit/exit;
- 12,000 output lines without a frozen UI;
- two connected hosts with preserved, isolated terminal canvases and scrollback;
- disconnect overlay, trusted reconnect, fresh session identity, and explicit stale-ID rejection;
- process restart with persisted hosts/trust but zero terminal sessions, followed by a fresh working PTY;
- removal of both fixture hosts and zero remaining encrypted credential rows.

The final profile database observation after host deletion was: 0 host rows, 0 credential ciphertext rows, 1 vault initialization marker, and 1 deliberately retained known-host pin. Logs contained 0 occurrences of all nine sanitized terminal-output markers and did not contain the fixture password. The native result record is [goal-02a-native-results.json](goal-02a-native-results.json). Raw native captures remain local validation evidence and are excluded from source publication because they include live workstation and fixture details.

Native validation used the built release because the computer-use entrypoint could not initialize its kernel assets in this environment. WebView2 CDP and Playwright operated the actual Tauri window and did not substitute a browser preview or mocked backend.

Final cleanup verified that the isolated native profile and fixture scratch path no longer existed, `NexusOps-Goal02A-20260914` was absent from WSL, loopback port 20196 had no listener, and no validated release process remained.

## Performance observations

This was practical stress verification, not a benchmark. The manager test filled all 64 output slots, proved the producer remained blocked while the consumer lagged, then drained it without exceeding the configured capacity. OpenSSH delivered 2 MiB from a fast `yes` pipeline through the real bounded path. The native app rendered 12,000 numbered lines, continued responding, switched active tabs and hosts immediately, ran two PTYs together, and propagated a viewport resize. No observed UI freeze, runaway queue, output crossover, or orphan terminal followed close/disconnect/shutdown.

The evidence does not establish a maximum sustainable throughput, memory high-water mark, CPU budget, or behavior under days of continuous output. Rapid manual boundary dragging and platform DPI transitions beyond the tested WebView resize were not instrumented. The explicit queue, poll, scrollback, terminal-count, timeout, and debounce limits provide bounded behavior for this milestone.

## Focused security review

| Area | Result and evidence |
| --- | --- |
| Tauri commands | Seven terminal-specific commands; no generic command executor. Arbitrary `execute_command` remains absent. |
| IPC capability | Main bundled window only; exact terminal commands and clipboard read/write text. No filesystem, process, shell, clipboard image/HTML/clear, or remote-window scope. |
| Session identifiers | Core verifies the current `HostSessionId`; manager independently verifies host, connection, and terminal IDs. Policy errors are safe and content-free. |
| Host isolation | Rust ownership regressions, frontend mapping tests, real two-PTY OpenSSH test, and native two-host switch all passed. |
| Clipboard | Explicit user action only, text only, no focus-triggered reads, no logging/persistence. |
| Sensitive logging | Source review found no payload logging. Native logs contained none of nine terminal markers and no fixture password. |
| Persistence | No terminal schema or storage path exists; xterm screen/scrollback and labels are memory-only. Restart returned zero sessions for both saved hosts. |
| Renderer injection | Bundled-only navigation, restrictive CSP, React text rendering, and prototype freezing remain enabled. xterm consumes bytes as terminal control data rather than HTML. |
| Resource exhaustion | 16 KiB input/chunk, 64 queue slots, 64 KiB poll, 10,000 lines, eight active terminals/host, and operation deadlines. Backpressure test passed. |
| Stale sessions | Disconnect/reconnect changes connection identity. Rust and native stale-ID attempts were rejected. |

`npm audit` reported 0 vulnerabilities. `cargo-audit` 0.22.2 scanned 652 locked dependencies and returned 0 vulnerabilities plus the same 7 allowed warnings recorded at Goal 01A: six unmaintained `unic-*`/`proc-macro-error` packages and the target-dependent `glib 0.18.5` iterator unsoundness warning. No warning originated in the terminal code. `glib` is not in the final Windows runtime path; the warning remains relevant to the existing Linux Tauri dependency graph. actionlint 1.7.12 returned no workflow diagnostics.

The renderer and system clipboard necessarily hold active content in memory. A compromised local account, renderer, debugger, clipboard manager, memory dump, malicious remote server, or remote shell history can expose it. This is documented and is outside an application-level claim of secrecy.

## Bugs discovered and fixed

1. PTY request acknowledgements could be interleaved with initial prompt bytes. Startup now buffers data until both server replies arrive.
2. A channel returned during a narrow disconnect/create race could have escaped teardown. Accepting the channel and changing state now occur under coordinated entry locks; canceled arrivals close immediately.
3. Normal remote terminal completion left the channel reference until a later action. The output pump now releases and closes it when it ends.
4. A disconnect arriving just after remote close could leave the tab labeled closed. Explicit connection teardown can now promote the ended record to disconnected.
5. Periodic frontend list refresh could replace a local rename during an in-flight response. Workspace recovery lists once on mount; per-session polling owns subsequent state.
6. Creating or selecting a host while already in the Terminal section could select it without mounting its workspace. Both host selection paths now register the host workspace.
7. Tauri prototype freezing exposed xterm 6's compiled assignment through inherited `Object.prototype.toString`, leaving the release window blank. A version-pinned Vite transform defines that one namespace property directly and fails closed if the expected source changes; prototype freezing stays enabled.
8. The first OpenSSH terminal test used a four-byte window for a three-byte UTF-8 symbol and wrote `\033` as a Rust null escape. Correct byte-width and `\x1b` assertions now exercise the intended data.

## Remaining limitations and unverified areas

- No PTY resume, tmux integration, persisted tabs, scrollback export, configurable scrollback, or shared terminal sessions.
- No SFTP, file editing, remote clipboard files, jump hosts, agent forwarding, SSH certificates, or key-rotation UI.
- Ed25519 remains the verified client-key type; the Goal 01A RSA advisory decision is unchanged.
- macOS and Linux compiled in the workspace checks where applicable CI definitions exist, but this session did not run native UI/keychain/PTY validation on those operating systems.
- htop, nano, less, journalctl follow, Docker log follow, sudo prompts, F1-F12 applications, mouse-reporting terminal programs, TrueColor environment negotiation, and long-duration network interruption were not individually exercised. The persistent PTY and xterm architecture is designed for those byte sequences, but this report does not claim direct verification.
- Installers were generated but are unsigned; no installer installation/uninstallation test or external publishing occurred.

These limits do not block the Goal 02A definition of done.

## Exact validation commands

Commands ran from the repository root after dot-sourcing the task-local MSVC/Rust environment where shown:

```powershell
. <task-work>\build-env.ps1
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p nexus-core --locked terminals_bind_to_one_connection_and_reconnect_never_reuses_them
cargo test -p nexus-secrets --test platform_store platform_keychain_persistence_update_association_and_delete --locked -- --ignored --exact --nocapture
cargo run -p nexus-core --example export_protocol --locked
```

```powershell
$node = (Get-Command node).Source
$npmCli = 'C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js'
& $node $npmCli run typecheck
& $node $npmCli run lint
& $node $npmCli test -- --run
& $node $npmCli run build
& $node $npmCli audit --audit-level=low
& $node $npmCli run tauri -- build
```

```powershell
$run = '<task-work>\goal-02a-openssh'
.\tools\openssh-fixture\Setup-OpenSshFixture.ps1 -RunRoot $run
.\tools\openssh-fixture\Start-OpenSshFixture.ps1 -RunRoot $run
.\tools\openssh-fixture\Run-OpenSshInterop.ps1 -RunRoot $run
.\tools\openssh-fixture\Cleanup-OpenSshFixture.ps1 -RunRoot $run
& <task-work>\security-tools\bin\cargo-audit.exe audit --file Cargo.lock
& <task-work>\security-tools\bin\actionlint.exe
```

The native run launched `work\target\release\nexus-desktop.exe` with the exact Goal 02A isolated-profile marker and WebView2 remote debugging enabled, then ran the three recorded Playwright phases against `http://tauri.localhost/`. It used the real Tauri IPC, Windows Credential Manager, SSH transport, and OpenSSH PTYs.

## Build artifacts and source state

Final toolchain: rustc/cargo 1.98.1 for `x86_64-pc-windows-msvc`, Node 24.19.0, npm 10.8.2. Exact frontend terminal dependencies are xterm 6.0.0, FitAddon 0.11.0, SearchAddon 0.16.0, and Tauri clipboard JS 2.3.3/Rust 2.3.2.

| Artifact | SHA-256 |
| --- | --- |
| `work/target/release/nexus-desktop.exe` | `5c04d256bfb061ddad284e578a1600ab7913cbddc528562ec707a5a6f77d12fc` |
| `work/target/release/bundle/msi/NexusOps_0.1.0_x64_en-US.msi` | `52908813f918348f2d3190bd64e64156bde2fe3d703804960216cad409dfb659` |
| `work/target/release/bundle/nsis/NexusOps_0.1.0_x64-setup.exe` | `37c99072cc45c98a9591eecc97fa55f81b08aa0705bc5b1fe613fc6a585480fe` |

The repository has Git metadata but no commit. `git log -1` returned no revision and `git status --short` reports the entire source tree untracked, as it did for Goal 01A. No commit, push, tag, pull request, release, upload, or unrelated cleanup occurred.

The exact 125 build-input files under Cargo manifests/lock, package manifests/lock, `apps/`, `crates/`, and `packages/` are recorded in `docs/validation/goal-02a-build-inputs.sha256`, sorted by repository-relative path. Documentation, evidence, generated frontend output, dependencies, compiler output, and fixture scratch data are excluded. The manifest has SHA-256 `d305d99b8e75ae67caf923ae2987c2378ab2fe2d62bee8c46d9677b13c09bf9e`.

Automated verification, environment-backed OpenSSH/keychain integration, native Windows observation, and unverified areas are separated above. Goal 02B should add the SFTP file manager on this typed, host-scoped connection foundation without exposing a generic filesystem or shell API.

READY FOR GOAL 02B — SFTP FILE MANAGER

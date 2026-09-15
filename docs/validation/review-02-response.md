# REVIEW-02 terminal integration response

Date: 2026-09-15
Repository: <https://github.com/unrealbg/NexusOps>
Draft PR: <https://github.com/unrealbg/NexusOps/pull/1>
Reviewed base: `f4db3cb9346a90504b6844617214831a65b9774a`
Reviewed head: `dced1fe59477ebf931db0d3aea95620e94317cfa`
Corrected implementation head before this evidence-only document: `cd1befad48dc4e9aef890e7a7007260945c7676c`

The report commit necessarily follows the implementation commit named above. The exact immutable published branch head and hosted checks for that head are recorded in Draft PR #1 after the grouped push.

## Finding disposition

| Finding | Disposition | Files | Commit |
| --- | --- | --- | --- |
| NX-007 | Corrected | `TerminalWorkspace.tsx`, `TerminalWorkspace.test.tsx`, `terminalFlow.test.ts`, `terminal.md` | `cd1befa` |
| NX-008 | Corrected | `terminalFlow.ts`, `TerminalPane.tsx`, `terminalFlow.test.ts`, `TerminalWorkspace.test.tsx`, `terminal.md` | `cd1befa` |
| NX-009 | Corrected | `terminalFlow.ts`, `TerminalPane.tsx`, `terminalFlow.test.ts`, `TerminalWorkspace.test.tsx`, `terminal.md` | `cd1befa` |

NX-007 now gives each mounted terminal identity one polling consumer. `TerminalWorkspace` passes a memoized poll-update callback, while `TerminalPane` retains the full declared dependency list. Rename, search, tab selection and ordinary parent renders no longer abort a batch already removed from the backend queue. Unmounting or replacing the terminal identity still aborts the consumer, preserving deliberate close/disconnect discard behavior.

NX-008 treats every rejected terminal write as having unknown delivery status. The input queue stops, releases its byte accounting, discards all pending local bytes and rejects later input for that terminal. It never retries the ambiguous chunk. The UI explains that delivery may be incomplete and instructs the user to close the terminal and open a new session. Output polling remains available for inspecting remote state, and the aggregate queued/in-flight limit remains 256 KiB.

NX-009 classifies only `timeout`, `connection` and `terminalStream` poll failures as transient. A successful poll resets the consecutive-failure counter and clears the transient UI error. Three consecutive transient failures publish a failed terminal snapshot and stop. Every other typed failure is permanent; `notFound` for a removed session therefore publishes a failed snapshot and terminates after one request.

## Baseline reproduction

The new tests were added before production changes and executed against `dced1fe59477ebf931db0d3aea95620e94317cfa` with Node.js 24.19.0.

| Evidence layer | Baseline result | Observation |
| --- | --- | --- |
| Helper suite | FAIL, exit 1: 8 passed and 5 failed of 13 | All three ambiguous-write scenarios accepted `SECOND` after `FIRST` failed, allowing replay. Open→`notFound` polled twice instead of once and never published failed state. Three transient failures also ended without a failed snapshot. |
| Component suite | FAIL, exit 1: 6 passed and 2 failed of 8 | Rename/search renders created 18 poll calls while the first 64 KiB xterm batch was awaiting callbacks; the test expected one. Open→`notFound` displayed the request error but never rendered `Terminal failed`. |

The component output regression mounts the real `TerminalWorkspace` and `TerminalPane` components with protocol and xterm boundaries mocked. It supplies four 16 KiB chunks with a unique marker at the end, holds each xterm write callback, performs rename and search UI interactions, then releases the callbacks. It checks byte-for-byte exact-once output, marker presence, one consumer during the batch, bounded polling after completion and emulator disposal only at unmount.

The input fault-injection cases model rejection before acceptance, after possible partial acceptance and after full acceptance with a lost acknowledgement. These prove calls made to the writer, not remote shell execution. Their harmless `FIRST` and `SECOND` markers are never executed as commands.

## Corrected automated evidence

All commands below ran from the project root after the final product-code change. Node commands used Node.js 24.19.0 with its directory first in `PATH`; npm was 10.8.2.

| Command | Exit | Result |
| --- | ---: | --- |
| `node node_modules/vitest/vitest.mjs run --config apps/desktop/vite.config.ts apps/desktop/src/features/terminal/terminalFlow.test.ts apps/desktop/src/features/terminal/TerminalWorkspace.test.tsx` | 0 | 22 of 22 helper/component tests passed. |
| Same focused command, five consecutive runs | 0 | 22 of 22 passed in every run. |
| `npm test -- --reporter=dot`, three consecutive runs | 0 | 36 of 36 passed in every full-suite run. |
| `npm run typecheck` | 0 | PASS. |
| `npm run lint` | 0 | PASS. |
| `npm run build` | 0 | PASS; the existing greater-than-500-kB chunk warning remains visible. |
| `cargo fmt --all -- --check` | 0 | PASS. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | PASS. |
| `cargo test --workspace --locked` | 0 | 68 tests passed; four platform/OpenSSH tests remained ignored by this default command. |
| `cargo run -p nexus-core --example export_protocol --locked -- --check` | 0 | PASS. |
| `npm run tauri -- build --no-bundle` | 0 | PASS; Windows release executable produced. |
| `Test-FixtureSafety.ps1` | 0 | 7 ownership-safety cases passed. |
| `npm audit --audit-level=low` | 0 | 0 vulnerabilities. |
| `cargo-audit audit --no-fetch --file Cargo.lock` | 0 | 0 vulnerabilities and the existing 7 allowed warnings. |
| `actionlint .github/workflows/ci.yml` | 0 | No diagnostics with actionlint 1.7.12. |

One intentionally unsupported invocation allowed npm's child processes to select the machine-default Node.js 22.7.0. The full suite then started no tests and reported seven `ERR_REQUIRE_ESM` worker errors at the CommonJS/ESM boundary. No install, dependency change or `node_modules` repair was made. Putting the supported Node.js 24.19.0 directory first in `PATH` made three complete 36-test runs pass. This environment failure is not counted as a product test failure or as passing evidence.

## Real OpenSSH integration

A new marker-owned Alpine 3.24 WSL2 fixture with OpenSSH 10.3p1 was created under a task-owned disposable root and exposed only on Windows loopback. No production host, production credential or existing WSL distribution was used.

The first phase-A attempt returned `Refused` before SSH because setup configures but does not start the listener. Inspection confirmed the missing listener. After invoking the documented `Start-OpenSshFixture.ps1`, the same fixture produced:

- `openssh_phase_a`: PASS, 1 of 1.
- `openssh_terminal_pty_interoperability`: PASS, 1 of 1, including the existing 2 MiB bounded PTY path.
- `openssh_phase_b_changed_key`: PASS, 1 of 1 after owned fixture key rotation.

Ownership-safe cleanup then removed only the new run root and unregistered only its recorded distro. The allowed parent remained. Pre-existing WSL distributions and the previously retained `active-fixture.json` were left untouched.

## Native Windows terminal acceptance

Status: **UNVERIFIED**.

Candidate source SHA: `cd1befad48dc4e9aef890e7a7007260945c7676c`
Candidate executable SHA-256: `CB34FE3E133C7B0C38740DE44401157180ADF976F685FE50DEF9E8C80C7D54C8`
Candidate executable size: 16,474,112 bytes
Build environment: Windows 11 Pro, version 10.0.26200, build 26200; Node.js 24.19.0; disposable loopback Alpine/OpenSSH fixture.

Computer Use was reset and initialized twice after the final product build. Both attempts failed before any application window could be observed or controlled:

`failed to write kernel assets: The system cannot find the path specified. (os error 3)`

No frontend preview, Rust PTY test, historical screenshot or alternate UI automation is presented as native evidence.

### Owner checklist required to clear the blocker

Use a fresh marker-owned fixture and isolated application profile. Keep fixture credentials and terminal contents out of screenshots, logs and the review report.

1. Check out the published REVIEW-02 head, build `nexus-desktop` in release mode, and record `git rev-parse HEAD` plus `Get-FileHash -Algorithm SHA256` for the executable.
2. Create a new owned scratch root. Run `Setup-OpenSshFixture.ps1`, then `Start-OpenSshFixture.ps1`. Read the loopback port and fingerprint from `fixture-metadata.env` and the disposable credential from `fixture-secrets.env`; do not copy either file into the repository.
3. Create a fresh absolute application profile containing `.nexusops-isolated-test-profile` with exactly `nexusops-goal-02a-isolated` followed by LF. Set `NEXUSOPS_TEST_APP_DATA_DIR` to it before starting the release executable.
4. Add the loopback fixture as a password host. Connect, verify the unknown-key fingerprint against fixture metadata before trusting it, then connect again and open a real PTY.
5. Produce more than 64 KiB of harmless numbered output ending in a unique marker. While xterm is still rendering, open/cancel rename, search for an earlier marker, switch tabs and return. Confirm every sequence number and the final marker appear once and in order.
6. Paste a harmless marker through the explicit Paste action and verify it once. Start a harmless repeating output loop, press `Ctrl+C`, and confirm the remote foreground process stops while the PTY remains usable.
7. Resize the window repeatedly. Run `stty size` after the final resize and verify its rows/columns. Open `top` or `vim`, enter and exit its full-screen mode, and confirm repaint/input remain correct.
8. Open two independent terminals and create a second disposable host record for the same fixture endpoint. Emit distinct harmless markers in each. Switch hosts and tabs and confirm no output, input, labels or scrollback cross between them.
9. Close one terminal while output is active and confirm its remaining output is deliberately discarded without affecting its sibling. Disconnect and reconnect the host; confirm the old PTY remains ended and cannot accept input, while a newly opened PTY works.
10. Exit NexusOps, start the same release again with the same isolated profile and confirm no old terminal tab or PTY is restored. Remove the disposable host records through the application.
11. Exit the application, run the ownership-safe cleanup script for the new fixture, and verify only its run root and distro are gone. Record sanitized pass/fail observations for every step, the Windows/WebView2 versions and the source/executable hashes.

Until a person or working native interaction runtime executes and records all scenarios, this required acceptance check remains a blocker.

## Publication state

Immediately before publication, repository visibility was verified as `PUBLIC` and was not changed. PR #1 remained Open and Draft against base `f4db3cb9346a90504b6844617214831a65b9774a`; auto-merge remained disabled. Corrected hosted run links and their exact published head are added to the PR after the grouped push rather than predicted in this commit.

No merge, base synchronization, force-push, history rewrite, reviewer-thread action, protection change, SFTP work or new feature work was performed.

NOT READY — BLOCKERS REMAIN

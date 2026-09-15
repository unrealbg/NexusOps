# REVIEW-03 terminal batch-boundary response

Date: 2026-09-15
Repository: <https://github.com/unrealbg/NexusOps>
Draft PR: <https://github.com/unrealbg/NexusOps/pull/1>
Reviewed base: `f4db3cb9346a90504b6844617214831a65b9774a`
Reviewed head: `c3de40a7067dba75efbdb5d706d1bcb68ef445e6`
Corrected implementation head before this evidence-only document: `fd418db47cb34ec0227b8d4131d6d36bf9d842d5`

The report commit necessarily follows the implementation commit named above. The exact immutable final branch head and hosted run links for that head are recorded in Draft PR #1 after the grouped push.

## NX-010 disposition

**Corrected.** `TerminalManager::poll()` previously tested only whether the bytes already collected were below 64 KiB before removing the next complete output chunk. The actual manager therefore returned 65,537 decoded bytes for the valid sequence `1 + 16,384 + 16,384 + 16,384 + 16,384`. The renderer correctly rejected the oversized batch after consuming 49,153 bytes, but the backend had already removed the rejected final chunk.

The corrected manager owns an `OutputBuffer` containing the existing bounded receiver and at most one carry-over suffix. When the next chunk crosses the 65,536-byte boundary, `poll()` returns the fitting prefix and retains only the suffix, which is less than 16 KiB. The next poll consumes that suffix before newer receiver data. Returned chunks remain at most 16 KiB, and every batch remains at most 64 KiB. `outputDrained` requires the sender and receiver to be drained and the carry-over to be absent. Explicit close and disconnect clear both the receiver and carry-over, retaining their documented discard behavior.

The queue remains 64 slots of at most 16 KiB. The only additional buffering is one suffix smaller than 16 KiB, so memory remains bounded. No frontend limit, protocol limit or dependency changed.

Files changed by implementation commit `fd418db`:

- `crates/nexus-terminal/src/manager.rs`
- `crates/nexus-terminal/src/manager/tests.rs`
- `apps/desktop/src/features/terminal/terminalFlow.test.ts`
- `crates/nexus-ssh/tests/openssh_terminal.rs`
- `docs/architecture/terminal.md`

NX-007, NX-008 and NX-009 production behavior and regression coverage were retained.

## Baseline reproduction

The supplied diagnostic archive had SHA-256 `81BA1350984E8D425D620C7E340927EDD9AF8759F2856C35A3EB8F0081649A01`. Its README and recorded Node harness output were treated only as external diagnostic material. They describe a source-derived JavaScript model and do not constitute execution of the Rust manager, OpenSSH transport or native application.

A regression was added to the actual `nexus-terminal` manager tests before changing production code. Against reviewed source `c3de40a7067dba75efbdb5d706d1bcb68ef445e6`, this command reached and ran exactly one product test:

```text
cargo test -p nexus-terminal --locked manager::tests::poll_keeps_irregular_chunks_within_the_batch_byte_limit -- --exact --nocapture
```

Exit code: `101`. Result: `0 passed; 1 failed; 9 filtered out`. The assertion reported `poll returned 65537 bytes; maximum is 65536`.

An earlier invocation without the configured MSVC environment stopped at the missing linker, and an invocation using an incomplete `--exact` filter ran zero tests. Neither is counted as baseline reproduction or product evidence.

## Corrected deterministic and frontend evidence

All commands in this section ran from the repository root with the supported Node.js 24.19.0 environment and the task-specific MSVC/Rust build environment.

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo test -p nexus-terminal --locked` | 0 | 13 of 13 manager tests passed. |
| `node node_modules/vitest/vitest.mjs run --config apps/desktop/vite.config.ts apps/desktop/src/features/terminal/terminalFlow.test.ts apps/desktop/src/features/terminal/TerminalWorkspace.test.tsx` | 0 | 23 of 23 helper/component tests passed. |
| `npm test` | 0 | 37 of 37 frontend tests passed. |
| `npm run typecheck` | 0 | TypeScript passed. |
| `npm run lint` | 0 | ESLint passed. |
| `npm run build` | 0 | Production frontend build passed. |
| `cargo fmt --all -- --check` | 0 | Formatting passed. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | Clippy passed with warnings denied. |
| `cargo test --workspace --locked` | 0 | 72 tests passed; 4 environment tests were ignored. |
| `cargo run -p nexus-core --example export_protocol --locked -- --check` | 0 | Protocol drift check passed. |
| `npm run tauri -- build --no-bundle` | 0 | Windows production desktop build passed; the command also produced the configured local bundles. |
| `npm audit --audit-level=low` | 0 | 0 vulnerabilities. |
| `cargo-audit audit --no-fetch --file Cargo.lock` | 0 | 0 vulnerabilities and the existing 7 allowed warnings. |
| `actionlint .github/workflows/ci.yml` | 0 | No diagnostics with actionlint 1.7.12. |
| `tools/openssh-fixture/Test-FixtureSafety.ps1` | 0 | 7 safety cases passed. |

The manager regressions simultaneously check the 64 KiB batch limit, 16 KiB chunk limit, exact bytes, order, absence of duplication and final `outputDrained` state. They cover the 65,537-byte minimal case, an exact 64 KiB irregular batch, several consecutive batches mixing small and maximum-sized chunks, carry-over followed by newer output, an EOF marker spanning the split, and explicit close/disconnect while carry-over is held.

The frontend regression passes the corrected 64 KiB irregular first batch and its one-byte carry-over through the actual `writeConsumed()` helper. It verifies exact reconstruction and a final marker. The existing component test continues to hold real xterm callbacks across rename/search renders and checks exact-once output, preserving NX-007 coverage. Existing fail-stop and lifecycle tests preserve NX-008 and NX-009 coverage.

The production frontend build retains the known 668.16 kB JavaScript chunk-size warning. It is not caused by this change.

## Real OpenSSH integration

A new marker-owned Alpine 3.24 WSL2 fixture with OpenSSH 10.3p1 was created under an isolated disposable root and exposed only on Windows loopback. The run used no production host, profile or credential.

- `openssh_phase_a`: PASS, 1 of 1.
- `openssh_terminal_pty_interoperability`: PASS, 1 of 1, including the 2 MiB PTY output path and assertions that every observed decoded output chunk was at most 16 KiB and every batch was at most 64 KiB.
- `openssh_phase_b_changed_key`: PASS, 1 of 1.

The real server cannot be required to choose a particular SSH packet segmentation, so the deterministic manager regressions remain the authoritative NX-010 boundary proof. The OpenSSH test confirms the same limits for every batch it actually observes and preserves the final volume marker.

Ownership-safe cleanup removed only the new run root and its recorded WSL distribution. The allowed parent remained. After WSL localhost forwarding settled, the test port had no listener or Windows listen record. The pre-existing `active-fixture.json` remained byte-for-byte unchanged.

## Native Windows acceptance

Status: **UNVERIFIED**.

Candidate source SHA: `fd418db47cb34ec0227b8d4131d6d36bf9d842d5`
Candidate executable SHA-256: `652D1D59E9866E4D0EF5122553006AE0CD752D092D773616AC9E75F97ACCF341`
Candidate executable size: 16,476,160 bytes
Build environment: Windows 11 Pro, version 10.0.26200, build 26200; Node.js 24.19.0; disposable loopback Alpine/OpenSSH fixture.

One Computer Use initialization was attempted after the final product build. It failed before any application window could be observed or controlled:

`failed to write kernel assets: The system cannot find the path specified. (os error 3)`

No additional initialization loop or alternate automation was used. A fresh isolated profile with the exact approved Goal 02A ownership marker was prepared locally. The owner must run the native scenarios in the REVIEW-02 checklist against a newly created marker-owned fixture: real SSH/PTy; output beyond one batch with a final marker during rename/search/tab changes; copy/paste and Ctrl+C; resize and full-screen TUI; two isolated terminals/hosts; close/disconnect/reconnect; and no old PTY after restart. Private launch paths and exact setup/cleanup commands are supplied outside this public report.

## Publication state

Immediately before publication, repository visibility was verified as `PUBLIC` and was not changed. PR #1 remained Open and Draft against base `f4db3cb9346a90504b6844617214831a65b9774a`; auto-merge remained disabled. The exact final evidence-only branch head, PR merge ref and successful hosted run links are added to the Draft PR after the grouped push rather than predicted in this commit.

No merge, force-push, history rewrite, reviewer-thread action, protection change, SFTP work or new feature work was performed.

CODE FIX PUBLISHED — NATIVE ACCEPTANCE PENDING — NOT MERGED

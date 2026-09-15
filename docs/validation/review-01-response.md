# REVIEW-01 correction evidence

Date: 2026-09-15  
Repository: <https://github.com/unrealbg/NexusOps>  
Draft PR: <https://github.com/unrealbg/NexusOps/pull/1>  
Reviewed base: `f4db3cb9346a90504b6844617214831a65b9774a`  
Reviewed head: `9996ec2a0d33d0caaf72cb0df6d201053dcc5464`  
Corrected implementation head before this evidence-only document: `51b5d03f85fc0ca0224276d8ef6a308b56bc71e2`

The document commit necessarily follows the implementation head named above. The exact immutable published branch head and the hosted checks for that head are recorded in Draft PR #1 after the grouped push.

## Disposition

| Finding | Disposition | Correction and regression evidence | Commit |
| --- | --- | --- | --- |
| NX-001 | Corrected | The protocol now exposes `outputDrained` separately from lifecycle completion. Backend polling reports it only after the output sender is closed and the queue is empty. The renderer keeps polling an ended session until that signal and waits for each xterm write callback before requesting the next batch. `remote_completion_drains_every_output_batch_before_signalling_completion` sends more than 64 KiB, closes the producer before the first poll, checks exact bytes and a final marker, and observes termination. Frontend flow tests independently cover multiple batches, callback backpressure, order, the final marker and termination. Explicit tab close and disconnect remain discard operations. | `82b63ca`, `b33420c` |
| NX-002 | Corrected | SSH terminal startup enforces one aggregate 64 KiB budget across PTY and shell acknowledgement waits, including both `Data` and `ExtendedData`. Exact-limit input is accepted; an additional byte returns a typed `TerminalStartup` error. Startup failure attempts a bounded channel close and never logs buffered terminal bytes. Tests cover early prompt data, the shared exact/over-limit boundary, continuing data without acknowledgement, cancellation and protocol failure. | `82b63ca` |
| NX-003 | Corrected | `TerminalManager::open` now routes outer timeout and connector rejection through cleanup instead of escaping through `?`. A startup guard cancels and removes the inserted entry if the caller future is dropped. Deterministic 1 ms timeout, repeated rejection and caller-abort regressions verify that no entry remains `Creating`, the error category is preserved, the eight-session capacity is not exhausted, and a replacement terminal can open. | `82b63ca` |
| NX-004 | Corrected | `terminalFlow.ts` serializes output into at most 16 KiB xterm writes and does not poll another backend batch until write callbacks consume the current batch. Cancellation releases a stalled callback wait. `BoundedTerminalInput` admits complete input events only while aggregate queued plus in-flight bytes remain at or below 256 KiB; rejected events are reported visibly and are not truncated. Tests use controlled slow/stalled consumers and writers to prove the bounds, resumption, cancellation and exact ordering of accepted bytes. Terminal bytes remain outside React state, persistent stores and logs. | `b33420c` |
| NX-005 | Corrected | Session merging now assigns label ownership to rename, lifecycle/error ownership to polling and dimensions to the mounted emulator. Ended lifecycle cannot regress to open, and identical idle snapshots preserve object identity. Per-session mutation revisions ignore stale rename completions and post-cleanup poll results. Controlled promises cover poll-before-rename, two out-of-order renames, poll completion after cleanup and a removed ended session that reports once and terminates. The rename assertion remains enabled. | `b33420c` |
| NX-006 | Corrected | The OpenSSH harness no longer imports the original task-local `work/build-env.ps1`. Every operation receives explicit `AllowedRoot` and `RunRoot` values. Canonical relative-path validation requires a dedicated strict child, rejects roots/outside paths/prefix collisions, and rejects reparse traversal. A versioned ownership marker includes a random ownership ID, roots, install directory, distro name and WSL registry identity. Cleanup verifies the marker and the registered distro's exact name, base path and registry key before unregistering or deleting. Seven mocked negative/positive safety tests passed, followed by a real disposable Alpine/OpenSSH setup, key-rotation run and cleanup from `E:\repos\NexusOps`. | `51b5d03` |

## Baseline evidence

The reviewed baseline was preserved at `9996ec2a0d33d0caaf72cb0df6d201053dcc5464`; it was not reset, amended or rewritten.

- NX-005 has dynamic baseline evidence. Hosted PR run [34889602513](https://github.com/unrealbg/NexusOps/actions/runs/34889602513) failed on Windows with 17 of 18 frontend tests passing at the retained rename assertion. The macOS and Ubuntu jobs passed. The separate push run [34889518775](https://github.com/unrealbg/NexusOps/actions/runs/34889518775) passed on all three operating systems. This establishes the intermittent state-ordering symptom without treating the earlier mock-isolation explanation as proven.
- NX-001, NX-002, NX-003, NX-004 and NX-006 were established by source reasoning against the reviewed commit. Their missing protocol states, byte budgets, cleanup path, consumer acknowledgements, aggregate input bound and portable ownership checks did not have safe, deterministic failing seams on that commit. No runtime failure is claimed for those five baseline cases.
- The corrected regressions are non-vacuous: they transfer bytes from active producers, force multiple batches or actual state transitions, and assert exact boundaries and terminal conditions.

## Local automated results

The applicable final gates were run from `E:\repos\NexusOps` after the final product-code change.

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test --workspace --locked` | PASS: 68 tests; 4 platform/integration tests ignored by the default command |
| `cargo run -p nexus-core --example export_protocol --locked -- --check` | PASS |
| `npm run typecheck` | PASS |
| `npm run lint` | PASS |
| complete `npm test` / Vitest suite | PASS: 27 of 27; repeated three times after the frontend corrections |
| focused terminal frontend suite | PASS: 13 of 13; repeated five times |
| `npm run build` | PASS; production Vite build, with the existing greater-than-500-kB chunk warning visible |
| `npm run tauri -- build --no-bundle` | PASS on the final product code; Windows executable built at the external target path |
| `Test-FixtureSafety.ps1` | PASS: 7 of 7 safety cases |
| `actionlint .github/workflows/ci.yml` | PASS with actionlint 1.7.12 |
| `cargo-audit audit --no-fetch --file Cargo.lock` | PASS with cargo-audit 0.22.2: 0 vulnerabilities and the existing 7 allowed warnings |
| `npm audit --audit-level=low` | PASS: 0 vulnerabilities |

The Rust total consists of 60 library tests plus 8 loopback integration tests. The four tests ignored by the default workspace run are one Windows credential-store test and three real OpenSSH tests; ignored tests are not counted as passes.

## Clean install and build environment

The machine-default Node.js 22.7.0/npm 10.8.2 attempt was outside the repository's Node requirement and then failed while npm tried to write its log because the configured `F:` volume had no free space. That attempt did not establish a Rolldown defect. The historical npm/Rolldown issue remains recorded in the earlier validation documents.

A documented clean `npm ci` then passed with Node.js 24.19.0 and npm 10.8.2, using the bundled Node executable and a new task-local npm cache. It installed 249 packages and reported 0 vulnerabilities. The Rolldown optional-binding issue did not reproduce in that clean supported environment; no manual `node_modules` repair was used as evidence.

Local Rust and Tauri builds still depend on machine tooling outside the project root: the MSVC linker/Windows SDK environment loaded by `C:\Users\jack\Documents\Codex\2026-09-14\files-pasted-by-the-user-goal\work\build-env.ps1`, the bundled Node.js 24.19.0 runtime, task-local Cargo/npm caches and the external Cargo target directory. Local audit validation also used task-local `actionlint.exe` and `cargo-audit.exe`. These are validation-machine dependencies and are not imported by the tracked OpenSSH fixture scripts.

`git diff --check` reports the generator-produced trailing space in `packages/protocol/src/index.ts` at `chunksBase64: Array<string>, `. The protocol consistency check passes and the generated file was not manually edited away from its generator. The production build warning for the 667.25 kB JavaScript chunk also remains visible.

## Real OpenSSH integration evidence

The portable harness ran against a disposable Alpine WSL distribution created under:

`C:\Users\jack\Documents\Codex\2026-09-14\files-pasted-by-the-user-goal\work\review01-openssh`

The unique distro name was `NexusOps-OpenSSH-201b4b61f442400688b7ea59540b9ec7`, bound only to loopback port 26244. The downloaded Alpine archive SHA-256 was `de9a11c0e0e7e9c94db3ed8af7b450eafc0b13687bd7e9199d55050f20aa0a89`.

- `openssh_phase_a`: PASS, 1 of 1.
- `openssh_terminal_pty_interoperability`: PASS, 1 of 1.
- After fixture key rotation, `openssh_phase_b_changed_key`: PASS, 1 of 1.
- Cleanup verification: the owned run root no longer exists, the allowed parent still exists, and the disposable distro is no longer registered. Existing Ubuntu and Docker Desktop distributions were untouched.

The remaining `active-fixture.json` in the disposable allowed parent is transient test metadata outside the repository. It contains no source or credentials and is not used by the completed fixture.

## Native Windows UI evidence

The required native Windows terminal smoke is **UNVERIFIED**. The Windows executable built successfully, but both attempts to initialize the desktop computer-use session failed before application interaction with:

`failed to write kernel assets: The system cannot find the path specified. (os error 3)`

The real OpenSSH PTY test verifies the Rust transport and terminal manager against a real server. It does not replace native xterm rendering, keyboard/paste, tab and lifecycle interaction in the packaged Windows UI. This unexecuted required check remains a blocker.

## Publication state and limitations

Before the grouped push, GitHub reported repository visibility `PUBLIC`; it was verified and not changed. PR #1 remained open and Draft, with base `main` at `f4db3cb9346a90504b6844617214831a65b9774a`, head branch `review/goal-02a-baseline`, and auto-merge disabled. The corrected hosted run results for the exact published head are added to the PR after publication rather than predicted here.

Remaining blockers and limitations:

- Native Windows terminal smoke could not run because the computer-use runtime failed to initialize.
- Hosted checks for the corrected head must complete after the grouped publication push; a pending, canceled or failed job remains non-passing.
- The existing dependency audit warnings and Vite bundle-size warning remain visible; this review does not perform unrelated dependency or bundling work.
- Caller-future cancellation is covered at the manager boundary, including capacity release. It does not claim proof that every buffer inside third-party SSH/OS layers is synchronously destroyed.

NOT READY — BLOCKERS REMAIN

# SFTP-REVIEW-01 response

- Date: 2026-09-15
- Branch: `feat/goal-02b-sftp`
- Reviewed head: `a132c2f2c0fea7cbddf32d2d6f28d2d57ab9b086`
- Accepted base: `a3fe2e3157660ca5c851fc3a9cddf7bd5645172e`
- Corrective product/test commit: `245ae2bd7b8ea9901c6c05ced5558e2be7cd1dca`
Draft PR: <https://github.com/unrealbg/NexusOps/pull/2>

## Result

The corrective implementation addresses SF-01 through SF-08 in source and adds deterministic regression coverage at the manager, application, local-grant and component boundaries. A fresh real OpenSSH run also passed the new nonuniform 256 MiB integrity check and a 5,001-child directory cap check. One final standalone release executable received native WebView2, verified OpenSSH, PTY, large-list and reconnect checks.

Real native picker/grant/transfer acceptance could not be repeated because the Windows Computer Use helper failed to initialize twice with `failed to write kernel assets: The system cannot find the path specified. (os error 3)`. The helper was reset once as required and the retry failed identically. No substitute driver or mock was counted. Several requested destructive/fault scenarios also remain unverified. The resulting publication verdict is:

`IMPLEMENTATION PUBLISHED — BLOCKERS REMAIN — NOT MERGED`

This is an implementation report, not approval to merge or begin Goal 02C.

## Baseline evidence

The reviewer explicitly reported source findings rather than a Windows runtime reproduction. The corrective run did not rewrite the reviewed commit or add tests to it. Inspection of exact reviewed head `a132c2f...` confirmed these call paths:

| Finding | Reviewed source evidence |
| --- | --- |
| SF-01 | `transfer.rs:116` acquired `jobs` before `order`; `trim_history()` at `transfer.rs:392` acquired them in the opposite order. |
| SF-02 | `run_upload()` called `upload_staged()` at `transfer.rs:521` and treated every returned failure as owned staging; the public client result at `client.rs:42` carried no ownership state. |
| SF-03 | `retry_plan()` at `transfer.rs:166` admitted `OutcomeUnknown` without enforcing `retryable`. |
| SF-04 | `execute()` admitted upload/download items through the single-item `enqueue()` at `transfer.rs:272`, allowing an earlier item to start before a later capacity error. |
| SF-05 | Finalization selected overwrite behavior from the batch policy at `transfer.rs:551` and `transfer.rs:644`, while planning represented an absent target only as `expected=None`. |
| SF-06 | `LocalItem` at `policy.rs:20` stored a path and metadata; `GrantValue::DownloadDirectory` at `local_access.rs:18` stored only a `PathBuf`; `StoredGrant` at line 20 had no connection scope. |
| SF-07 | `open_sftp()` at `application/files.rs:6` separated lookup, async startup and insert without per-host coalescing. |
| SF-08 | Rename planning at `policy.rs:278` discarded the identity read during approval, and transfer completion did not revalidate both source sides after streaming. |

The unconfigured shell initially failed before source compilation because `link.exe` was unavailable. All Rust results below use the same task-local environment loader for the repository-pinned Rust/Node toolchain plus MSVC and Windows SDK paths. That loader is outside the project root and is not a product dependency; a normal Developer PowerShell with the prerequisites in `docs/development/setup.md` is equivalent.

## Finding disposition

### SF-01 — resolved in source and deterministic tests

`TransferManager` now owns one `Mutex<ManagerState>` containing both the job map and display order. Listing, state transitions, capacity accounting and history trimming use that single state. No manager state lock is held across SFTP or local I/O. `listing_and_state_updates_share_one_lock_with_a_watchdog` uses a controlled barrier plus a two-second watchdog; `completed_history_is_strictly_bounded` proves the exact 100-terminal-entry contract.

### SF-02 — resolved in source and injected failure tests

`StagingOwnership` is shared between the transfer guard and `SftpClient::upload_staged()`. A client implementation marks ownership immediately after a successful exclusive `OPEN` response. Failures are typed as `NotCreated`, `OwnedFailure`, or `CreationOutcomeUnknown`. Cleanup is possible only for confirmed ownership and only for the exact staging path. The RAII guard covers post-stream identity failure, cancellation and caller future drop; a lost create reply performs no removal. Tests distinguish pre-existing staging, unknown creation, owned failure, cleanup failure after commit attempt and caller drop by inspecting remove calls and preserved content.

A process loss or unknown create reply can still leave an orphan. NexusOps deliberately does not guess ownership or wildcard-delete `.part` files.

### SF-03 — resolved in manager and core tests

The backend rejects generic retry for `OutcomeUnknown` and for `Failed` jobs whose `retryable` flag is false. Cancelled and retryable failed work are replanned through `FilePlanStore::plan_upload()` or `plan_download()`, which rechecks the current SFTP session, source, destination, capabilities and local handles. The old payload is not cloned into a new plan. Manager tests and `core_retry_boundary_rejects_unknown_and_permanent_outcomes_without_io` prove that rejected retries issue no identity I/O and that an allowed retry obtains new preconditions.

### SF-04 — resolved with atomic batch admission

`enqueue_batch()` builds the bounded job set, locks manager state once, checks the whole nonterminal capacity and inserts all accepted records before spawning workers. Capacity failure accepts zero items and starts zero I/O. The controlled regression holds every global worker permit, fills 99 of 100 slots, submits two-item batches in parallel, proves no new job was admitted, cancels one held job, and then proves exactly one new item can be admitted.

### SF-05 — resolved for CreateNew; Replace limitation documented

Planning now records `DestinationAction` per item: `Skip`, `CreateNew`, or `ReplaceExisting(expected identity)`. An absent target always becomes `CreateNew`, including under a batch Replace preference. Upload uses the negotiated no-clobber remote commit and download uses `persist_noclobber`; a target that appears after approval is preserved. Replace is available only for a specifically observed regular target and still revalidates that identity.

Unit tests cover upload and download competitors with distinguishable content, per-item action selection and same-destination serialization. OpenSSH capability tests retain the hardlink/POSIX-rename requirements. SFTP v3 provides no universal compare-and-swap for replacement of an existing remote file, so existing-target Replace remains vulnerable to an undetectable same-metadata race by a malicious server or writer.

### SF-06 — implemented; native picker rerun blocked

Upload grants retain opened regular-file handles. On Windows those handles allow read sharing but deny write/delete sharing; transfer reads a cloned handle and checks handle/path identity before and after streaming. Download grants retain an opened directory handle, reject symlink/reparse components across the complete ancestry, and validate that handle before temporary creation and finalization. The `NamedTempFile` handle is retained with its cleanup path and is not reopened by name.

Every local grant is typed, one-shot, expiring and scoped to `HostId + HostSessionId + SftpSessionId`. Picker commands capture that scope before the OS dialog, validate it again afterward and revoke pending grants on disconnect/reconnect. The frontend passes the captured session into picker commands. The approved download path is included in user-visible plan text but remains excluded from logs and committed evidence.

Windows tests prove selected-file write/rename denial, selected-directory rename denial, grant A rejection against host/session B, expiry/one-shot behavior and session revocation. Component coverage proves a delayed picker retains its original remote target and passes the captured SFTP ownership. Real picker, junction-parent replacement, temp-parent replacement and reconnect-while-picker-open could not be rerun because the Windows UI helper was unavailable.

### SF-07 — resolved with controlled lifecycle tests

Application state now holds a per-host async startup gate. `open_sftp()` acquires that gate across current-client recheck, startup, generation validation and publication. Simultaneous opens coalesce to one connector call and the same published client. `close_sftp()` uses the same gate and removes only a subsystem owned by the requested SSH connection, so late cleanup for an old generation cannot close a new one. Controlled connector barriers cover simultaneous opens and disconnect between startup and publication; a late old cleanup leaves a reconnected session intact.

### SF-08 — resolved within available identity signals

Rename plans retain the approved remote source identity and execution compares it before mutation. Upload reads the retained local handle and validates source metadata both before and after streaming. Download validates remote source identity before and after streaming. Detected changes prevent finalization, and existing destinations remain untouched.

Tests cover changed rename identity, Windows source replacement/write denial, and a same-size remote source whose modification time changes during download. SFTP v3 commonly exposes only kind, size and second-resolution modification time. A malicious server can change bytes while reporting unchanged metadata; NexusOps does not claim snapshot semantics against that server.

## Validation results

All successful commands exited 0. Rust commands were prefixed by the task-local toolchain/MSVC environment loader described above.

| Layer | Command | Result |
| --- | --- | --- |
| Rust format | `cargo fmt --all -- --check` | PASS |
| Rust compile | `cargo check --workspace --all-targets --locked` | PASS; existing `ts-rs` transparent-attribute warnings visible |
| Rust lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| Rust full suite | `cargo test --workspace --all-targets --locked` | PASS; 101 passed, 5 opt-in tests ignored by the broad command |
| Protocol drift | `cargo run -p nexus-core --example export_protocol --locked -- --check` | PASS |
| Frontend types | `npm run typecheck` | PASS |
| Frontend lint | `npm run lint` | PASS |
| Frontend tests | `npm test -- --run` | PASS; 8 files, 40 tests; existing jsdom canvas notice visible |
| Frontend production build | `npm run build` | PASS; 91 modules; existing 681.83 kB chunk advisory visible |
| Native standalone build | `& '.\\node_modules\\.bin\\tauri.cmd' build --no-bundle` | PASS |
| Fixture safety | `tools/openssh-fixture/Test-FixtureSafety.ps1` | PASS; 7 cases |
| Real keychain | `cargo test -p nexus-secrets --test platform_store --locked -- --ignored --nocapture` | PASS; 1 test and cleanup |
| Workflow lint | `actionlint` 1.7.12 | PASS |
| JavaScript audit | `npm audit --audit-level=low` | PASS; 0 vulnerabilities |
| Rust audit | `cargo audit` 0.22.2 | PASS; no vulnerability failure, 7 allowed warnings |

The seven cargo-audit warnings are unchanged: `proc-macro-error` (`RUSTSEC-2024-0370`), five `unic-*` crates (`RUSTSEC-2025-0081`, `-0075`, `-0080`, `-0100`, `-0098`) and target-conditional `glib` (`RUSTSEC-2024-0429`). No warning was suppressed or added to an ignore list.

The first hosted macOS run exposed a test-fixture portability error: `tempfile::tempdir()` selected `/var/folders`, while the product's ancestry policy correctly rejects macOS `/var` because it is a symlink. The failing core test exited 101 before retry I/O. The tests now pass the canonical path of each disposable OS temp resource into the ancestry-sensitive API, so macOS uses `/private/var/folders`. Product behavior was not relaxed, and tests do not leave temporary directories in the checkout. The full local Rust unit/bin suite, format and Clippy then passed again before the correction was pushed.

The first direct opt-in SFTP command exited 101 before SFTP work because phase A had not yet created the fixture pin; its assertion was `phase A must establish trust first`. The documented sequence was then used: exact phase A passed in 16.74 s, followed by `openssh_sftp_streaming_interoperability`, which passed in 309.18 s. It covered actual SFTP v3 negotiation, Unicode/binary/empty cases, no-clobber/Replace/rename/cancellation, simultaneous PTY, the nonuniform 268,435,456-byte upload/download with independent streaming SHA-256 and offset checks, and an actual 5,001-child directory reported as exactly 5,000 entries with `partial=true`.

Two npm wrapper attempts intended as `--no-bundle` builds produced installers because npm consumed the flag. They exited 0 but were not used for native identity. The documented direct local CLI command then produced the standalone executable without bundling.

## Native evidence and artifact identity

The final standalone executable built from product/test commit `245ae2bd...` has:

- Size: 17,811,968 bytes
- SHA-256: `7A29AE59D5588C7A72723C9F8C7C95D3DCFBAF55639AA30F97187EEB99943CE0`

That exact executable ran with an isolated marker-owned profile, real WebView2, the fresh disposable OpenSSH server and the actual Windows credential store. It passed host creation, explicit fingerprint trust, real PTY input/output, Files subsystem startup, a real 5,001-entry directory capped at `5000+ loaded`, filtering to an exact row, enabled refresh, disconnect/reconnect without another trust prompt, and no stale transfer jobs. The large listing reached the visible capped state in 1,397 ms on this run. The table remains non-virtualized; this is a single responsiveness observation, not a benchmark or a guarantee for slower machines.

No native upload/download picker action is claimed for this executable. Computer Use initialization failed twice with the same local helper error, so the real picker, multi-file batch, Keep both and picker-after-reconnect scenarios remain blockers. Browser mocks and direct IPC were not substituted for the picker/grant boundary.

## Remaining unverified scenarios

- Real native multi-file batch, Keep both, picker cancellation after reconnect, and transfer approval showing the full approved local destination.
- Disposable Windows junction/reparse ancestry replacement and temp-parent replacement during a real transfer.
- Real symlink delete with distinguishable target content.
- Denied write, disk-full, close failure and ambiguous finalization across every setup/stream/finalize stage.
- Active-transfer disconnect through the final native UI and setup/caller-drop handle cleanup at the application boundary.
- A universal existing-target Replace compare-and-swap, which cannot be promised by the negotiated SFTP v3 primitives.

The deterministic tests do cover the corresponding policy boundaries for grant scope/revocation, atomic admission, caller-drop staging cleanup, non-retryable unknown outcomes, source revalidation and no-clobber competitors. They are reported separately from missing native/fault evidence.

## Cleanup and publication state

The native process, isolated profile, large-list remote directory and marker-owned OpenSSH fixture are removed after evidence capture. No credential, local path, payload, screenshot, trace, installer or fixture is committed. The final repository secret/diff checks and hosted CI/test-merge identity are recorded at publication time. Merge performed: **No**. Auto-merge enabled: **No**. Goal 02C started: **No**.

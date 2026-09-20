# SFTP-REVIEW-02 response

- Date: 2026-09-16
- Branch: `feat/goal-02b-sftp`
- Reviewed head: `ba9bd7fc4b5e563122487064b658d06970dcdd16`
- Accepted base: `a3fe2e3157660ca5c851fc3a9cddf7bd5645172e`
- Corrective code head: `9e598246100eaff66ffbd89f9e9e40fbd9773eb4`
- Corrective commits: `ac434a64dcec3923475fddae6a9480dc3c53ce56`, `9e598246100eaff66ffbd89f9e9e40fbd9773eb4`
- Corrective test merge: `ecb632a60d1462e03a48797c41161c50a6b52ad3`
- Draft PR: <https://github.com/unrealbg/NexusOps/pull/2>

## Result

SF-09, SF-10 and SF-11 are corrected in source and covered by focused runtime regressions. The remaining SF-06 resource-lifetime paths now have explicit discard, idle expiry and terminal-state release behavior. Active disconnect cleanup and Unix symlink deletion also have controlled regressions. The final code head passed the hosted Windows, Ubuntu and macOS quality matrix.

The real Windows picker scenarios were not executed. Native automation was unavailable, and an attempted isolated native process start was rejected before execution by the automation approval policy. No process or profile was created, and mocked grants or direct transfer IPC were not counted as picker evidence. The controlled disk-full/permission/remote-close matrix and several native race scenarios also remain incomplete. The publication verdict is therefore:

`IMPLEMENTATION PUBLISHED — BLOCKERS REMAIN — NOT MERGED`

This report does not approve merge or Goal 02C.

## Baseline runtime evidence

The reviewed commit was preserved. Temporary tests were applied only in a detached worktree at exact head `ba9bd7fc...`, executed, and removed with that worktree.

| Finding | Baseline result |
| --- | --- |
| SF-09 | Two distinct disposable four-byte Windows files were assigned identical creation and modification times. The old `(creation_time, file_size)` surrogate compared equal, while `fsutil file queryfileid` returned distinct IDs: `0x0000000000000000000200000028d1f0` and `0x0000000000000000000100000028d1f1`. |
| SF-10 | A controlled upload reached `Completed` while its history row remained. Renaming its local source failed with Windows sharing error 32, proving that the terminal `JobRecord.spec` still held the file handle. The temporary regression exited 101. |
| SF-11 | A controlled download plan returned only `remote-source.bin` for `destination_display` instead of the expected full selected directory and final filename. The temporary regression exited 101. |

These results demonstrate the reported boundaries. They do not claim remote compromise or an independently reproduced exploit.

## Finding disposition

### SF-09 — resolved

`LocalIdentity` now separates mutable `size`/`modified` preconditions from immutable `LocalObjectId { storage_id, file_id }`. Windows identities come from an opened handle through the safe `fs-id` wrapper, retaining the volume serial and full 128-bit file identifier. Unix retains filesystem device and inode identity. The Windows-only dependency is target-scoped because `fs-id 0.2.0` does not compile on the current macOS libc; the Unix implementation uses `std::os::unix::fs::MetadataExt` directly.

The code fails closed when a strong handle identity cannot be obtained. Source and directory paths are reopened without following a final Windows reparse point and compared with the retained selection handle. Destination serialization keys use the selected directory object ID plus final child name.

`windows_object_identity_uses_volume_and_full_file_id` creates different files with equal explicit creation time, modification time and size, then proves different object IDs. It also proves that two observations of one object match, that object identity survives a permitted content mutation while the complete precondition changes, and that distinct directories differ. `destination_keys_use_object_identity_and_final_name` covers directory identity and final-name serialization.

### SF-10 — resolved for implemented lifetime boundaries

`JobRecord` keeps execution authority in `active_spec: Option<JobSpec>`. The worker drops its task-owned spec before publishing a terminal state, and `Completed`, `Cancelled`, `Failed` and `OutcomeUnknown` all clear `active_spec`. Terminal history retains display, counters, outcome and error metadata only. All terminal transfer records are non-retryable; retry planning now requires a new picker selection/grant, and generic retry continues to reject `OutcomeUnknown`.

`FilePlanStore` and `LocalAccessService` now use shared stores with per-record Tokio expiry tasks. A pending plan expires after five minutes and a local grant after ten minutes even when the app is otherwise idle. Tests use paused Tokio time and configurable short lifetimes rather than real sleeps. Scoped `discard_file_plan` and `discard_local_grant` commands remove only the named resource in the matching host/SSH/SFTP scope. If consume wins the mutex race, the active operation owns the handles and discard is a no-op for that consumed record.

Approval Cancel and modal close notify the backend. Partial preparation/execution failures also discard unused plans. The permissions manifest and generated Tauri command permissions include only the two narrow discard commands.

Focused manager and service regressions prove that source handles are released for every terminal state while the history row remains, a completed download releases its destination directory, pending approval discard releases handles, idle expiry releases handles, and another active lease remains held. `active_disconnect_cancels_once_cleans_owned_staging_and_releases_source` proves single cancellation, confirmed-owned staging cleanup and source release after disconnect.

### SF-11 — resolved

Download approval derives its escaped display from the immutable selected directory plus the final planned filename. This includes the actual Keep both suffix. `download_approval_displays_full_final_destination` proves that equal filenames in different selected directories produce distinct full displays and that Keep both shows its selected final name. Component coverage proves that approval Cancel invokes scoped plan discard.

The local path remains user-facing approval data. It is not added to audit logs, fixture output, screenshots or committed runtime evidence.

### Residual SF-06 scope

The implementation retains opened upload files and download directories only while a grant, pending plan or active transfer owns them. Session changes revoke pending grants; transfer history cannot restore authority. Existing no-reparse ancestry checks, handle/path revalidation, one-shot ownership and typed host/SSH/SFTP scope remain intact.

Real picker cancellation, reconnect while a picker is open, Windows junction/ancestor replacement during transfer and native post-completion rename still require owner execution. The deterministic layers validate the backend invariants but are not reported as OS picker acceptance.

## Validation

All reported successful commands exited 0. Rust commands used the repository-pinned toolchain with the installed MSVC/Windows SDK environment. That environment setup is a build prerequisite rather than a product runtime dependency.

| Layer | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test --workspace --lib --bins --locked` | PASS; 101 tests |
| `cargo test -p nexus-sftp --lib --locked` | PASS; 30 tests on Windows |
| `cargo test -p nexus-ssh --test loopback --locked` | PASS; 9 tests |
| Protocol drift check | PASS |
| `npm run typecheck` and `npm run lint` | PASS |
| `npm test` | PASS; 8 files, 41 tests; existing jsdom canvas notice visible |
| `npm run build` | PASS; 91 modules; existing chunk-size advisory visible |
| Direct `tauri build --no-bundle` | PASS |
| Platform keychain ignored integration | PASS; 1 test and cleanup |
| OpenSSH fixture safety | PASS; 7 cases |
| `actionlint` 1.7.12 | PASS |
| `npm audit --audit-level=low` | PASS; 0 vulnerabilities |
| `cargo audit` 0.22.2 | PASS; 0 vulnerabilities and 7 allowed warnings |

The cargo-audit warnings remain visible: `proc-macro-error` (`RUSTSEC-2024-0370`), five `unic-*` advisories (`RUSTSEC-2025-0081`, `-0075`, `-0080`, `-0100`, `-0098`) and target-conditional `glib` (`RUSTSEC-2024-0429`). No ignore entry or safety suppression was added.

A fresh marker-owned OpenSSH fixture passed phase A in 16.78 seconds, PTY interoperability in 11.97 seconds, SFTP interoperability in 309.65 seconds and phase B in 0.06 seconds. The SFTP run retained the 256 MiB nonuniform integrity round trip, large-list cap, cancellation, conflict and simultaneous terminal coverage. The disposable distro, listener, remote content and coordination files were removed and independently checked after the run.

The first hosted run for `ac434a6...` exposed two portability issues rather than a product-runtime failure: an unused Windows-only test helper under Linux Clippy and an upstream `fs-id` Unix build failure on macOS. Commit `9e59824...` target-scoped that dependency, used standard device/inode identity on Unix and target-scoped the helper. Push Quality run <https://github.com/unrealbg/NexusOps/actions/runs/35024226784> and pull-request Quality run <https://github.com/unrealbg/NexusOps/actions/runs/35024232756> then passed on Windows, Ubuntu and macOS for exact code head `9e598246...`. GitGuardian also passed.

## Final Windows candidate

The standalone executable was rebuilt after the last code change from `9e598246100eaff66ffbd89f9e9e40fbd9773eb4`:

- Size: 17,736,192 bytes
- SHA-256: `8F271F4E1EF286493E751DAED8D57744E27DBFE8B48E4BA97DD41C2472397FBD`

No executable, installer, profile, local path, credential, raw payload, screenshot or fixture is committed.

No native picker action is claimed for this candidate. Source/unit/component/manager tests and real OpenSSH interoperability are recorded separately from the missing Windows picker evidence.

## Owner native checklist

Use a locally rebuilt executable from code head `9e598246...` or verify the exact size and SHA-256 above. Use a marker-owned isolated profile, disposable local OpenSSH fixture and non-production credentials. Record PASS/FAIL for each item and remove the process, profile, listener, remote content and credentials afterward.

1. Use the real OS pickers for multi-file upload and download; confirm the approval shows the full final local destination.
2. Exercise Skip, Keep both and Replace, including preservation of old content after an injected failure.
3. Cancel the picker and approval modal; verify selected files/directories can immediately be renamed.
4. Open a picker, reconnect before accepting it, and verify that the stale grant cannot start work.
5. Complete a transfer; keep its history row visible and verify the local file/directory can be renamed.
6. Disconnect during an active transfer; verify one cancellation, owned cleanup and no replay.
7. Delete a disposable symlink and verify its distinguishable target remains unchanged.
8. Run a terminal concurrently and restart NexusOps; verify no active transfer is restored.

## Remaining limitations

- Real OS picker multi-file, conflict, cancel, reconnect and visible-full-target flows remain UNVERIFIED.
- Native post-terminal file/directory unlock and active-transfer disconnect remain UNVERIFIED, despite deterministic Windows handle and manager coverage.
- Windows junction/reparse ancestor replacement and temp-parent replacement during a real transfer remain UNVERIFIED.
- Controlled disk-full, permission-denied and remote-close failures are not yet covered across every setup, stream and finalization boundary. Tests cover confirmed-owned cleanup, caller drop and ambiguous creation without claiming the full matrix.
- Unix symlink deletion is covered by a disposable runtime test; the equivalent real Windows link flow remains UNVERIFIED.
- Existing-target Replace cannot provide a universal compare-and-swap guarantee with the negotiated SFTP v3 primitives.

## Publication and cleanup state

Corrective code head `9e598246...` is published to the existing branch and Draft PR. Base `a3fe2e3...` and test merge `ecb632a...` were read back from GitHub. The detached baseline worktree and marker-owned OpenSSH resources were removed. The standalone executable remains an untracked build output ignored by Git. Merge performed: **No**. Auto-merge enabled: **No**. Goal 02C started: **No**.

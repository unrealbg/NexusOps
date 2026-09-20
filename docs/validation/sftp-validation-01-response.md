# SFTP-VALIDATION-01 response

- Date: 2026-09-17
- Repository: `unrealbg/NexusOps`; branch: `feat/goal-02b-sftp`; Draft PR #2
- Starting head: `dfe70e320c5d8f84cc5c90cc1ded1871c3d0b791` (clean working tree)
- Validated code/test head: `0360ef67b62139692738adcfa375b5eee393c593`
- Evaluated base: `a3fe2e3157660ca5c851fc3a9cddf7bd5645172e`
- Code/test-head GitHub test merge: `7fa304c7e5fb22d5af0f8bc1976cb9d6c1aa9e60`

This validation added deterministic SFTP protocol fault injection and manager checks to the existing Goal 02B branch. The previously accepted SF-09, SF-10 and SF-11 corrections and the earlier SSH, terminal and SFTP tests remain in place. `run_download_with_staging` is a behavior-equivalent extraction of the existing local temporary-file creation so the creation failure can be injected without filling a disk or changing another directory's permissions. No product defect was confirmed by the new regressions, so no other application behavior was changed.

## Bounded fault matrix

All new protocol tests exercise `RawSftpClient` over an in-memory SFTP v3 transport. The fixture records real request packets and keeps distinguishable in-memory file contents. The lost-reply cases decode and encode actual SFTP packets, execute the mutation in the fixture, then close the transport before replying. Unit/manager tests use disposable local directories and controlled notifications; no test touches a production host, user ACL, or system disk capacity. The client's 15-second request timeout and explicit 2–5-second test watchdogs bound waits.

| Phase and injection | Evidence and result | Bytes, outcome, files and calls |
| --- | --- | --- |
| A: exclusive `OPEN` denied with preexisting staging | New `denied_exclusive_open_preserves_preexisting_staging_and_final`; PASS | `NotCreated(SftpDenied)`, ownership false, 0 bytes, one OPEN, no REMOVE. The preexisting staging and final remain byte-equal to their sentinels. |
| A: exclusive create executes but reply is lost | New `lost_exclusive_create_reply_never_establishes_ownership_or_removes_staging`; existing manager `staging_cleanup_requires_confirmed_ownership`; PASS | `CreationOutcomeUnknown`, ownership false, 0 confirmed bytes. The fixture has a created staging object, but the client sends no WRITE/REMOVE or replay; the final sentinel is intact. Manager cleanup is forbidden without confirmed ownership. |
| A: local staging creation fails | New `failed_local_staging_creation_starts_no_remote_read_or_final_file`; PASS | Controlled `StorageFull` from the actual temporary-file creation seam after source identity check. `LocalAccess`, 0 downloaded bytes, one identity call, zero remote downloads. Disposable directory remains empty; no final or staging file. |
| B: second remote WRITE is denied | New `denied_second_write_keeps_confirmed_progress_below_payload` and `plan_and_manager_fail_mid_upload_clean_owned_staging_without_commit`; PASS | First 65,536-byte chunk is confirmed; second WRITE is denied. Raw result is owned failure. Real `FilePlanStore` and `TransferManager` produce `Failed`, `confirmed_bytes=65536`, non-retryable. Exactly one owned REMOVE removes the partial staging; the old final and an unrelated preexisting staging sentinel remain intact. No commit request. |
| B: local writer accepts bytes, then reports disk full | New `download_closes_remote_handle_after_partial_local_disk_full`; PASS | The writer accepts six bytes, then returns `StorageFull` on the next chunk. Only six bytes are progress-confirmed; `LocalAccess` is returned and remote CLOSE occurs once. No finalization is invoked. |
| B: second remote READ is denied | New `denied_second_remote_read_closes_handle_after_nonzero_bytes`; PASS | Six bytes are read and confirmed before `PermissionDenied`. `SftpDenied`, one CLOSE, no final mutation; the final sentinel remains intact. |
| C: fsync, local flush, or remote close fails after payload | New `fsync_and_close_denials_do_not_commit_complete_payload` (two subcases) and `download_flush_failure_after_all_bytes_is_not_success`; PASS | Full upload payload is WRITE-confirmed before fsync/CLOSE denial, but `upload_staged` returns owned failure and sends no commit. Download accepts its full 12-byte payload, then flush returns `StorageFull`; `LocalAccess` follows one remote CLOSE. Progress is not treated as a committed final. |
| C: destination/source identity denied or changed | Existing `owned_staging_is_cleaned_on_post_stream_failures_and_caller_drop`, `finalization_uses_the_per_item_approved_action`, `changed_remote_source_blocks_download_finalization`, `download_replace_preference_cannot_overwrite_a_new_target`; PASS | A post-stream identity timeout or changed target/source prevents commit and cleans confirmed-owned staging. The CreateNew competitor and download competitor tests make zero overwrite commits and preserve the competing final. |
| D: competitor appears after CreateNew identity read, before hardlink | New `create_new_competitor_between_identity_and_hardlink_is_preserved`; PASS | The fixture creates the competitor as it answers LSTAT absent. The hardlink fails definitively; no REMOVE or retry follows. The competitor final and staging contents remain byte-equal to their sentinels. |
| D: final mutation is denied | New `known_commit_denial_preserves_final_and_confirmed_commit_cleanup_error_preserves_result` (denied Replace subcase); PASS | The real `posix-rename@openssh.com` request receives `PermissionDenied`, mapped to `SftpDenied`. Both old final and staging are intact. This is a confirmed failure, not `OutcomeUnknown`. |
| D: Replace executes, then reply is lost | New `executed_replace_with_lost_reply_is_outcome_unknown_without_replay`; PASS | Fixture final changes and staging disappears, but the client has no reply and returns `OutcomeUnknown`. Exactly one mutation request; no rollback, retry or claim that the old final survived. |
| D: hardlink commit succeeds, staging REMOVE is denied | New `known_commit_denial_preserves_final_and_confirmed_commit_cleanup_error_preserves_result` (cleanup subcase); existing `owned_staging_is_cleaned_on_post_stream_failures_and_caller_drop`; PASS | The final is confirmed to contain the staging content and the staging object remains. One REMOVE is denied. The current API conservatively reports `OutcomeUnknown` with an explicit message that the final file exists; that uncertainty concerns cleanup, not whether hardlink committed. No rollback of the final occurs. |
| E: active cancel, disconnect, caller drop | Existing `cancelled_stream_never_reaches_finalization`, `active_disconnect_cancels_once_cleans_owned_staging_and_releases_source`, `owned_staging_is_cleaned_on_post_stream_failures_and_caller_drop`; PASS | Active cancellation prevents finalization, disconnect cancels once and cleans owned staging, and task drop schedules owned cleanup. Existing Windows handle tests verify source release after terminal state. |
| E: cancel races finalization; other host/queue | New `cancel_after_finalization_starts_cannot_erase_confirmed_commit_or_other_host`; existing `queued_cancellation_starts_no_remote_io`, `cancelling_one_job_does_not_release_another_active_lease`, `jobs_for_one_destination_are_serialized`; PASS | Once the first job reaches `Finalizing`, cancel/disconnect cannot erase its confirmed commit. It completes after one commit and zero cleanup calls; the other host completes independently. Both execution specs clear and global/host semaphore permits return to capacity. Queued cancellation starts no remote I/O. |

The direct protocol tests assert `RawSftpClient` results and protocol calls. The manager tests assert `TransferManager` states, progress, cleanup and resource release. The local staging test invokes the actual download path with an injected failing creator; it does not claim that a real disk was filled. Earlier real OpenSSH interoperability, including a 256 MiB integrity round trip, remains evidence from SFTP-REVIEW-02; it was not rerun for this fault-injection-only matrix. None of these layers is native picker acceptance.

## Validation and candidate

The first unconfigured Rust probe exited 1 before test execution because `link.exe` was unavailable. This is not counted as a failed product test or as a pass. Subsequent Rust checks used the installed MSVC and Windows SDK compiler environment and repository-pinned Rust 1.98.1; Node checks used Node 24.19.0. Commands below exited 0 unless stated otherwise.

| Exact command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0; existing `ts-rs` serde-attribute notices remain visible |
| `cargo test --workspace --lib --bins --locked` | 0; 114 passed, 0 failed/ignored, including 43 `nexus-sftp` tests (13 new), with the final manager test rerun after its addition |
| `cargo test -p nexus-sftp --lib plan_and_manager_fail_mid_upload_clean_owned_staging_without_commit --locked` | 0; 1 selected, 1 passed |
| `cargo test -p nexus-ssh --test loopback --locked` | 0; 9 passed |
| `cargo run -p nexus-core --example export_protocol --locked -- --check` | 0; no DTO change or drift |
| `npm run typecheck`; `npm run lint` | 0 each |
| `npm test` | 0; 8 files, 41 tests; existing jsdom canvas notice |
| `npm run build` | 0; 91 modules |
| `node_modules/.bin/tauri build --no-bundle` | 0; build only, no desktop executable launched |
| `tools/openssh-fixture/Test-FixtureSafety.ps1` | 0; 7 cases |
| `git diff --check` | 0 |

The initial read-only executable candidate from SFTP-REVIEW-02 was present at `target/release/nexus-desktop.exe`: 17,736,192 bytes, SHA-256 `8F271F4E1EF286493E751DAED8D57744E27DBFE8B48E4BA97DD41C2472397FBD`. The no-bundle validation rebuilt it from code commit `a74c08aa2c10b6530dd34fb90054025e57a4b2fc`: 17,741,824 bytes, SHA-256 `5661FC4EC9A28C9B8FF9E7BDABEAB9E30D4F32D842E0A0A0E262D438614225A9`. Commits `d2a0f921700ea239edd1457543a045e24f1f433f` and `0360ef67b62139692738adcfa375b5eee393c593` change only tests. Neither candidate was run for native acceptance. No executable or trace is committed.

Disposable test directories are scoped to test lifetimes and removed; protocol files live only in memory. The fixture safety check created no OpenSSH distro or listener. There is no active native process or profile from this task. The old `active-fixture.json` was not touched.

## Native handoff and limits

Native start/control remains blocked by the prior `blocked by policy` decision. The blocked action was not retried through a shell, launcher, test hook or other path. A future authorized tester in a permitted environment should record PASS/FAIL against an exact source/artifact identity for: real multi-file upload/download pickers and full approval destination; Skip/Keep both/Replace; picker and approval cancellation with released files; picker after reconnect; Completed history without retained file locks; active disconnect; symlink target preservation; concurrent PTY; and restart without stale jobs. These are ordinary native flows, separate from the controlled fault injection. Every item remains **UNVERIFIED** here; no human is asked to reproduce disk-full or transport races.

The negotiated SFTP v3 operations still cannot provide a universal compare-and-swap for an existing Replace target. That documented protocol limitation is not presented as a passing test. Native acceptance and Goal 02B acceptance remain pending; this report does not authorize merge or Goal 02C.

## Hosted publication

The code/test head above was pushed to the existing branch for Draft PR #2. Hosted Quality runs for exact head `0360ef67b62139692738adcfa375b5eee393c593` both completed successfully: [push run 35220539341](https://github.com/unrealbg/NexusOps/actions/runs/35220539341) and [pull-request run 35220542797](https://github.com/unrealbg/NexusOps/actions/runs/35220542797), with Windows, macOS and Ubuntu quality jobs passing. Publishing this report creates a later documentation-only head and requires its own hosted CI check; the code-head result is not silently attributed to that later commit. The source is not merged, auto-merge is off, and PR #2 remains Draft.

`FAULT VALIDATION COMPLETE — NATIVE ACCEPTANCE PENDING — NOT MERGED` applies to the local headless matrix; hosted CI status is recorded separately above.

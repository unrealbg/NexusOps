# Goal 02B SFTP validation report

Date: 2026-09-15  
Branch: `feat/goal-02b-sftp`  
Accepted base: `a3fe2e3157660ca5c851fc3a9cddf7bd5645172e`  
Product/test source commit: `2d5ffb05cb4546b25ef826f68f125824d24ebb9b`  
Implementation commit: `0f37e32f3b11868615eaf45ded60e436cc37a709`  
Publication status at report creation: local commits only; hosted CI and PR test-merge SHA pending.

## Result

The review candidate implements a real SFTP v3 Files workspace on the existing authenticated SSH transport. It includes bounded browsing, metadata/properties, native upload/download selection, streamed file transfers, explicit conflict handling, create-directory, safe regular-file rename, non-recursive delete, cancellation, explicit retry plans, session ownership, and a transient transfer queue.

The implementation is suitable for source review, but the milestone remains blocked from a full acceptance verdict because several requested failure-first and native scenarios are not independently exercised. The exact verdict is:

`IMPLEMENTATION PUBLISHED — BLOCKERS REMAIN — NOT MERGED`

Publication is completed after this report is committed, pushed, and attached to a new Draft PR. This verdict never authorizes merge.

## Architecture and security contracts

- `russh-sftp` 3.0.0 opens a separate SFTP subsystem channel on the current verified `russh` transport. There is no second login, shell command, SCP, or terminal fallback.
- Every listing, plan, transfer, and command is bound to `HostId + HostSessionId + SftpSessionId`. Reconnect revokes plans and does not resume transfers.
- The renderer can request only native picker operations and receives an opaque typed grant with display metadata. Local paths and file bytes remain in Rust.
- File plans are backend-owned, immutable, capped, five-minute, single-use approvals. Local grants are typed, capped, ten-minute, and consumed during planning.
- Uploads and downloads stream in at most 64 KiB payload chunks. File data is never base64-encoded or buffered in React.
- New remote files and Keep both require `hardlink@openssh.com` v1. Remote Replace requires an existing regular file and `posix-rename@openssh.com` v1. Missing capability disables planning.
- Staging files are job-specific, created exclusively beside the destination, and use mode `0600` remotely. Existing final files are not deleted before replacement.
- Queued cancellation starts no remote I/O. Cancellation observed before finalization removes only the exact job staging file. Finalization cannot be relabeled Cancelled after a confirmed commit.
- Lost replies from create/delete/rename/new-file commit/Replace become non-retryable `OutcomeUnknown`. Refresh and a new decision are required; no ambiguous write is blindly replayed.
- SFTP unavailable/denied errors are typed and scoped to the subsystem; the verified SSH transport and PTYs remain independently usable.

## Limits

| Boundary | Limit |
| --- | ---: |
| Client SFTP packet | 256 KiB |
| File payload chunk | 64 KiB |
| Library concurrent reads/writes | 4 / 4 |
| SFTP request timeout | 15 s |
| Directory listing | 5,000 validated entries |
| Active transfers | 4 global, 2 per host |
| Nonterminal queue | 100 jobs |
| Completed in-memory history | 100 jobs beyond the active bound |
| Native upload selection | 32 regular files |
| Pending local grants | 64 |
| Pending file plans | 100 |
| Keep both candidates | 1,000 |

Rust byte counters and offsets use checked `u64`. Wire counters are decimal strings and frontend calculations use `BigInt`, including values above `2^53`.

## Local gates

The final deterministic gates ran from the repository root on Windows NT 10.0.26200.0 with Rust/Cargo 1.98.1, Node 24.19.0, and npm 10.8.2. All listed commands exited 0.

| Command | Result |
| --- | --- |
| `npm ci --include=optional` with Node 24 | Clean install; 249 packages installed; 0 npm vulnerabilities |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test --workspace --lib --bins --locked` | PASS; 74 tests |
| `cargo test -p nexus-ssh --test loopback --locked` | PASS; 9 tests, including typed rejected SFTP with SSH still usable |
| `cargo run -p nexus-core --example export_protocol --locked -- --check` | PASS; no generated DTO drift |
| `npm run typecheck` | PASS |
| `npm run lint` | PASS |
| `npm test` | PASS; 8 files, 40 tests |
| `npm run build` | PASS; 91 modules |
| `npm run tauri -- build --no-bundle` | PASS; production executable built |
| `cargo test -p nexus-secrets --test platform_store --locked -- --ignored` | PASS; real signed-in Windows keychain round trip and cleanup |
| `tools/openssh-fixture/Test-FixtureSafety.ps1` | PASS; 7 cases |
| `actionlint` 1.7.12 | PASS |
| `npm audit --audit-level=low` | PASS; 0 vulnerabilities |
| `cargo audit` 0.22.2 | Exit 0; no vulnerability failure, 7 allowed warnings listed below |

The final Rust SFTP unit set has 8 tests. It covers path/display policy, Windows aliases, plan ownership/replay/expiry, unsupported safe-finalization capability, Skip batch behavior, queued cancellation without remote I/O, and cancellation before finalization. Component tests cover delayed directory responses, picker target capture, unmount behavior, lossless progress above `2^53`, and visible transfer state.

The `ts-rs` generator emits its existing warning that it ignores `serde(transparent)` while generating newtype declarations. The checked-in protocol drift test remains authoritative and passed. Vitest emits jsdom's non-fatal canvas `getContext` notice. Vite emits a 681.81 kB minified chunk advisory (189.32 kB gzip); this is a size warning, not a failed gate.

## Real OpenSSH evidence

A fresh marker-owned Alpine 3.24.0 WSL2 fixture ran OpenSSH 10.3_p1-r1 as a non-root test account on loopback. The combined wrapper passed phase-A authentication/trust (16.75 s), the existing PTY interoperability test (10.84 s), the SFTP test (304.85 s), and phase-B changed-key handling (0.06 s).

After the SHA-256 assertion was added, a second fresh phase-A fixture reran the exact SFTP test from source commit `2d5ffb05cb4546b25ef826f68f125824d24ebb9b`; it passed in 309.71 s. The test negotiated SFTP v3, inspected advertised OpenSSH hardlink/POSIX-rename/limits extensions, browsed and statted, transferred Unicode/binary/empty files, preserved an old destination after refused no-clobber finalization, replaced safely, renamed, cancelled both before and during streaming, opened a simultaneous PTY, and uploaded/downloaded 268,435,456 bytes. The downloaded large file was checked byte-by-byte and by an independently accumulated streaming SHA-256 digest. No product-side remote hash command exists.

An earlier practical sample during the same 256 MiB loopback workload observed approximately 12.98 MB process working set. This is a single observation, not a throughput or memory benchmark.

Both fresh fixtures were cleaned through the ownership-checked script. Follow-up checks found no fixture directory, registered distribution, listener, or NexusOps process. Two failed wrapper attempts were retained as setup evidence rather than counted as product failures: one reused an already-rotated phase-B fixture while phase A was expected; the other omitted the documented fixture start step and received connection refused. The subsequent clean documented sequence passed.

## Native Windows evidence

The production Tauri executable was exercised through its actual WebView2, Rust commands, OS dialogs, keychain, and real OpenSSH SFTP subsystem. Browser-preview mocks were not used for native acceptance.

The broad native run exercised real native file/folder pickers, an 8-byte binary upload and independent local byte verification after download, Skip and Replace, create-directory, safe file rename, file/directory delete, simultaneous terminal input/output, preserved per-host location, reconnect, and cleanup. A final rebuilt release then rechecked the changed risk surface: restart/reconnect with no stale active jobs, picker-selected upload, approval text showing `Conflict: skip` without a local path, a cancelled 268,435,456-byte upload with no final destination, and switching between two independently configured app hosts. A final source rebuild received a normal launch check with no CDP/debug listener.

The product/test source commit is `2d5ffb05cb4546b25ef826f68f125824d24ebb9b`. The final local executable was 17,713,664 bytes with SHA-256 `E3399EBCF8C4C95BD84859EA9AC901973DE64A342803AFABC89F14D90F30A815`.

No screenshots, credentials, user file paths, or payload contents are published. The isolated native profiles, temporary payloads, WSL distributions, loopback listeners, and application processes were removed. The repository worktree was clean before this report was created.

## Audit warnings

`cargo audit` reported seven allowed warnings and exited 0:

- `proc-macro-error` 1.0.4: unmaintained (`RUSTSEC-2024-0370`).
- `unic-char-property`, `unic-char-range`, `unic-common`, `unic-ucd-ident`, and `unic-ucd-version` 0.9.0: unmaintained (`RUSTSEC-2025-0081`, `RUSTSEC-2025-0075`, `RUSTSEC-2025-0080`, `RUSTSEC-2025-0100`, `RUSTSEC-2025-0098`).
- `glib` 0.18.5: unsound `VariantStrIter` advisory (`RUSTSEC-2024-0429`). This target-conditional dependency is not present in the Windows binary, but remains visible in the cross-platform lockfile audit.

No warning was suppressed or added to an ignore list for this milestone.

## Known limitations and acceptance blockers

- Safe no-overwrite rename is limited to regular files on servers advertising `hardlink@openssh.com` v1. Directory and symlink rename are disabled because SFTP v3/OpenSSH exposes no negotiated primitive with the required no-clobber contract for those entry types.
- The 5,000-entry partial-list boundary and virtualization behavior were not exercised against a real large OpenSSH directory.
- Real/native evidence does not independently cover queue saturation, simultaneous same-destination jobs, a multi-file native batch, Keep both through the native UI, symlink deletion, disk-full/permission loss during finalization, caller-future drop, or disconnect during an active transfer.
- Fault-injection coverage does not yet demonstrate every setup/stream/finalize close failure, source replacement race, duplicate close, and ambiguous mutation response requested by the milestone.
- The broad native sequence and the final targeted delta checks span two local release builds. The final commit changes only integration test evidence and a dev dependency, and the exact final executable received a normal launch check, but every broad native action was not repeated again on that last executable hash.
- Hosted GitHub CI and the PR test-merge SHA were pending when this report commit was created. Their actual results must be read from the Draft PR; future success is not claimed here.

These gaps require the blocker verdict even though implemented behavior, deterministic gates, real SFTP round trip, keychain test, and targeted native checks passed.

Merge performed: **No**.  
Auto-merge enabled: **No**.  
Goal 02C started: **No**.

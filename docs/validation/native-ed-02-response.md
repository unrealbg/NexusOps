# NATIVE-ED-02 source response

Baseline: `26e56dc85a85c7deb73f6bf3dbaf5edb83bbecb1` on `feat/goal-02c-remote-editor`, base `c3939415b5980095f0acef3f60e2d4fe35c00180`. This response covers source and a separate disposable OpenSSH fixture. Owner native acceptance remains pending; no desktop executable was built or launched.

## Baseline reproduction

- Frontend: the new Cancel → Save / review regression failed because Save stayed disabled and the editor displayed its stale-authority warning. The dirty buffer remained intact.
- Backend: a second `plan_editor_save` after `discard` failed with `FilePolicy: The editor document is stale; reload it.`

## Corrected lifecycle

The document authority retains the original HostId, HostSessionId, SftpSessionId, path, revision, SHA-256 content digest, newline/BOM rules and one-hour expiry. Planning reserves the authority for one request. A successful plan holds one pending reservation and its bounded encoded payload for up to five minutes. Confirmed discard of that still-pending plan releases the reservation and drops the payload; the next plan uses the current editor buffer but rechecks the **original** revision and digest. It does not open a new baseline. A plan removed after expiry, execution, document retirement or session revocation cannot confirm Cancel.

Document and plan state transitions use the document-then-plan lock order. Execute permanently consumes the document before it enqueues the transfer. A dropped planning future releases only its own reservation; Close, expiry, replacement and session revoke remove the record, so delayed work cannot revive it. The existing SF-12 transfer barrier, staging ownership, metadata checks and OutcomeUnknown behavior are unchanged. Successful and ambiguous transfers still require explicit Reload before another save.

The renderer closes Cancel/X/Escape approval immediately, waits for the backend's boolean release confirmation, and blocks another review or Reload while waiting. Confirmed Cancel retains the dirty text and shows a neutral notice. False or failed discard preserves the text, reports that cancellation could not be confirmed, and blocks another save until Reload. Session turnover and Close invalidate delayed responses.

## Validation

- Focused FilesWorkspace/RemoteTextEditor: 40 tests passed. Regressions cover P1 Cancel → P2 without Reload, latest buffer snapshot, X/Escape, delayed and failed discard, reconnect, Close/unmount, and Cancel/Approve exact-once behavior.
- `cargo fmt --all --check`: pass.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: pass. The existing `ts-rs` serde-transparent parser notices remain.
- `cargo test --workspace --lib --bins --locked`: pass, including 73 `nexus-sftp` and 27 `nexus-core` tests. New policy tests cover same-size/same-mtime digest Conflict, parallel plans, Cancel/execute race, aborted and retired delayed planning, plan/document expiry, and more than 100 Cancel cycles without retained plans.
- `cargo test -p nexus-ssh --test loopback --locked`: 9 passed.
- `cargo run -p nexus-core --example export_protocol --locked -- --check`: pass.
- `npm run typecheck`, `npm run lint`, `npm test` (99 passed), and `npm run build`: pass. jsdom emitted its existing canvas notice; Vite emitted its existing large-chunk warning.
- Separate marker-owned OpenSSH fixture: phase A passed; the editor test passed LF/CRLF/BOM `plan → discard → new plan → execute → exact-byte` checks and 0644 preservation; the confirmed-owned staging disconnect cleanup test passed. The fixture distribution was unregistered and its run directory removed. No production host or owner fixture was used.

SFTP v3 revision/digest validation followed by POSIX rename is not an atomic server-side compare-and-swap. Native Windows owner acceptance of this source remains unverified. Hosted CI is reported against the exact published PR head in the PR description.

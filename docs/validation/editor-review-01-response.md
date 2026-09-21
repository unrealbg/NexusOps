# EDITOR-REVIEW-01 source response

Base: `c3939415b5980095f0acef3f60e2d4fe35c00180`; reviewed head: `685100e18301a4f4e7527a473b3f3791b375254d`. This is a source and disposable OpenSSH response only. **NATIVE ACCEPTANCE PENDING.** No native owner candidate was built or launched.

## Baseline regressions on the reviewed implementation

- ED-01: delayed `planTextSave` after confirmed Close/unmount left the plan undiscarded (zero scoped discard calls).
- ED-02: a plan for text A surfaced an approval after the visible buffer changed to B.
- ED-03: the scoped `discard_editor_document` test failed compilation because no document retirement operation existed.
- ED-04: Ctrl+S from an editor control produced zero plans; an AltGraph event was incorrectly prevented and planned.
- ED-05: the blocked final editor read observed `Finalizing` before commit validation finished.

## Corrected behavior

The editor retires pending async work on Close/unmount; late plans and ignored opens/reloads are discarded using their original SFTP scope. A plan is published only for the unchanged text and buffer generation that requested it. The backend has a narrow, scoped, idempotent document discard and an idle expiry task. Physical Ctrl+KeyS works inside the visible editor, while Alt/Meta/AltGraph/repeat and invalid editor states pass through. The final revision/content validation remains cancellable; `Finalizing` starts immediately before the replacement request. Raw SFTP metadata denial or mismatched readback aborts before replacement and cleans the confirmed-owned staging file. New nexus-core tests cover old document/plan rejection after reconnect, planning serialized against teardown, and delete/shutdown quiescence for an active editor save.

## Validation

Focused: FilesWorkspace/RemoteTextEditor 32 tests PASS; new nexus-sftp ED-03/ED-05 and Raw metadata fault tests PASS; new nexus-core editor lifecycle tests PASS.

Full local: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --lib --bins --locked` (67 nexus-sftp tests, 27 nexus-core tests), `cargo test -p nexus-ssh --test loopback --locked` (9), protocol drift check, `npm run typecheck`, `npm run lint`, `npm test` (91), and `npm run build` all PASS. Rust emitted the pre-existing `ts-rs` serde-transparent parser notices; jsdom emitted its canvas notice.

Disposable real OpenSSH: phase A trust/auth setup PASS; editor LF/CRLF/BOM round trip, 0644 mode preservation and independent replacement Conflict PASS; deterministic confirmed-owned staging disconnect cleanup PASS. The initial editor attempt before phase A was correctly refused as an unknown fixture host key. The marker-owned WSL distribution and scratch root were cleaned. Production hosts and native NexusOps execution were not used.

SFTP v3 metadata/content checks plus POSIX rename remain a best-effort concurrency boundary rather than universal atomic CAS. Native Windows owner acceptance remains pending source review.

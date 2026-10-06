# Goal 01 implementation plan

The initial workspace was empty. NexusOps is a new Rust workspace and npm workspace.

1. Record boundaries and security decisions; establish domain and typed IPC contract.
2. Implement isolated SSH sessions, explicit TOFU, persistent endpoint pins, timeouts and cancellation.
3. Implement platform credential storage and transactional host metadata persistence.
4. Add independently testable read-only probes, capability registry, operation policy and safe audit events.
5. Connect a Tauri 2 application service to a React host management and overview UI.
6. Run unit and local SSH integration tests, frontend checks, Clippy, production builds and desktop startup verification.
7. Document verified behavior, limitations, setup and CI.

## Risks to resolve

- Trust must bind the exact hostname, port, algorithm and fingerprint; stale prompts must not overwrite pins.
- Host edits and cancellation must not allow an old connection task to publish results into a new session.
- Secret storage errors must fail closed; metadata must contain no credential material.
- Untrusted remote output must be bounded, parsed as data and never logged or rendered as HTML.
- A failed optional probe must preserve the usable connection.
- Generated Rust/TypeScript contracts and tests must prevent IPC drift.
- Native OS dependencies and keychains differ; Windows is verified locally and other platforms have CI build coverage.

## Goal 05A remote-operation foundation

Goal 05A introduces `nexus-remote-operations` beside the permanently read-only operation engine. The internal crate owns one-shot native authority, a 120-second monotonic TTL, one pending plan per host, one executing operation per host, a process-wide execution limit of one, lifecycle revocation, fresh-revalidation hooks and dispatch-aware outcomes. It has no concrete production operation, persistence, IPC, frontend surface or SSH mutation transport.

Lifecycle work takes the host operation gate before the short application metadata gate and releases metadata/session locks before transport work. Execution admission is non-queuing. Consumed authority is never restored, including after `OutcomeUnknown`; compensation is operation-specific and may not exist. Audit adds the compatible `OutcomeUnknown` value and an explicit native-risk path while unknown legacy string kinds fail closed.

## Goal 05B bounded systemd reset-failed

Goal 05B implements the first and only production native operation: clear the failed-state marker for one backend-observed strict `.service` unit that is freshly revalidated as `loaded / failed / failed`. Opaque observation IDs and one-shot plan IDs remain memory-only and session/generation-bound. The renderer receives no operational target input; it can only plan, discard or execute the fixed operation after a separate review confirmation.

The exact SSH mapping has fixed environment and `systemctl --system --no-pager --no-ask-password reset-failed --` prefix, one validated target, bounded channel/request/cleanup time and 8 KiB combined output. It never uses sudo, generic command execution, retry or rollback. Audit truth and post-operation observation are separate from mutation truth. Source tests and loopback SSH tests are part of this implementation candidate; real systemd mutation and Native Windows acceptance remain separately authorized future work.

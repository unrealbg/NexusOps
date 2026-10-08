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

Goal 05B implements the first production native operation: clear the failed-state marker for one backend-observed strict `.service` unit that is freshly revalidated as `loaded / failed / failed`. Opaque observation IDs and one-shot plan IDs remain memory-only and session/generation-bound. The renderer receives no operational target input; it can only plan, discard or execute the fixed operation after a separate review confirmation.

The exact SSH mapping has fixed environment and `systemctl --system --no-pager --no-ask-password reset-failed --` prefix, one validated target, bounded channel/request/cleanup time and 8 KiB combined output. It never uses sudo, generic command execution, retry or rollback. Audit truth and post-operation observation are separate from mutation truth. Goal 05B disposable real-systemd acceptance and Goal 05C Native Windows desktop acceptance both passed on the merged implementation; Goal 05C made no source change.

## Goal 05D bounded systemd try-restart

Goal 05D adds the second production native operation. Only a strict backend-observed unit in exact `loaded / active / running` state receives a try-restart capability. Planning and execution each perform a fresh complete service inventory. The final fixed command remains `systemctl --system --no-pager --no-ask-password try-restart -- <STRICT_UNIT>`, so a unit that becomes inactive after revalidation is not started by the operation.

Try-restart is High risk, one-shot, non-queuing and bounded to one host target, 30 seconds of completion time and 8 KiB of aggregate output. It has a separate observation capability, plan/result DTOs, three operation-specific IPC commands and `service.try_restart` metadata-only audit. No ordinary restart, generic service verb, sudo, retry or rollback exists. Source review, source-policy re-review, disposable real-systemd try-restart acceptance and Native Windows desktop acceptance passed for the reviewed implementation merged as `4f9479cdff0da77a35116326d245cc4117e615f2`; post-merge main CI also passed. The production desktop path produced exactly one intended try-restart dispatch, one-shot replay and authority behavior remained fail-closed, and SSH infrastructure remained unaffected.


## Goal 05E bounded systemd reload

Goal 05E adds a third, sealed native operation for exactly one backend-observed strict service that is freshly revalidated as `loaded / active / running / CanReload=yes`. The inventory command now requests `CanReload`, parsed only from exact `yes` or `no`. Reload and try-restart retain separate opaque capabilities and may coexist on one row.

The only mutation mapping is `systemctl --system --no-pager --no-ask-password reload -- <STRICT_UNIT>` with fixed locale/environment controls, High risk, one target, 5/30/2-second transport bounds and 8 KiB combined output. It has separate plan/discard/execute IPC, result DTOs and `service.reload` metadata-only audit. There is no generic service verb, restart fallback, retry, rollback, sudo or multi-target form.

Source review, source policy, OpenSSH interoperability, disposable real-systemd reload acceptance and Native Windows desktop end-to-end acceptance passed for the implementation merged as `4addd8ab6bd42d4d3fc440db815a0c9e67d6c909`; post-merge CI also passed. Acceptance observed exactly one intended reload dispatch and one `ExecReload` execution. MainPID, InvocationID and the process start timestamp remained unchanged with `NRestarts=0`; SSH remained usable and no retry or restart fallback occurred.

# Remote operation authority foundation

Goal 05A added the native, memory-only foundation. Goal 05B uses it for exactly one production semantic operation, `SystemdResetFailed`, while `nexus-operations` and `RemoteSession::execute` remain permanently read-only. `nexus-remote-operations` stays transport-independent; the reviewed SSH adapter implements its typed mutation transport in one focused module.

## Authority ownership

An authority is minted from a sealed native operation type. `SystemdResetFailed` is the only production implementation. The stored record binds an unpredictable UUID plan ID to `HostId`, exact `HostSessionId`, session generation, native operation type, native `OperationRisk`, backend-owned target and preconditions, issue/expiry instants and a private native payload. It stores no credential, username or authentication method. Private targets, preconditions and payloads have no Serde or TypeScript representation.

The store permits one pending authority per host. Publishing a replacement atomically removes the previous authority. The fixed TTL is 120 seconds using Tokio's monotonic clock. Expiry is enforced both by a best-effort timer and synchronously whenever authority is observed or consumed. Authorities are not persisted and cannot survive process restart.

Planning publishes only when the backend binding captured after planning still exactly matches the draft binding. This prevents a delayed result from recreating authority after session turnover. Disconnect, reconnect, session replacement, host edit, host deletion, detected remote closure and application shutdown revoke matching pending authority. Shutdown seals the store atomically so later planning cannot republish authority.

Consume and discard compete under the same store mutex. Successful consume removes authority before revalidation and dispatch. Double consume fails, discard is scoped to the exact plan and binding, and expiry races at that same ownership boundary. Capacity rejection before consume leaves authority pending. Every later failure, cancellation or ambiguous result consumes it permanently; execution never restores a plan.

## Admission and lock ordering

Future execution uses a dedicated async gate per host and one global semaphore permit. Admission uses `try_lock` and `try_acquire`, so a user action returns Conflict when either capacity is unavailable; it is never queued for background execution. The initial bounds are one executing mutation per host and one executing mutation process-wide.

The lock order is:

1. acquire or try the host operation gate;
2. acquire global execution capacity for execution, or take the short `Application::mutation` metadata gate for lifecycle work;
3. capture or revalidate bounded session/metadata state;
4. release all short metadata/session locks;
5. perform fake or future reviewed remote I/O while retaining only execution admission.

Host lifecycle work waits for the host operation gate before it takes `Application::mutation`. It never holds the global metadata gate while waiting for the host gate. Connection establishment releases the gates before SSH I/O and relies on cancellation plus generation checks before publication. Disconnect and delete perform the bounded session transition under the metadata gate, release it, then quiesce SFTP/terminal/transport resources while retaining the host gate. Application shutdown seals remote-operation authority and performs its bounded state transition under the metadata gate, then releases that gate before resource I/O. Consequently a long operation or lifecycle wait on Host A does not monopolize unrelated metadata work.

Routine remote mutations will use ordinary process command admission. They must not call the updater-only `seal_and_drain_others()`. If updater sealing wins before command admission, the mutation cannot start. If a future mutation command was already admitted, updater sealing waits for its permit to leave.

## Revalidation and outcomes

After one-shot consume and before dispatch, the concrete service revalidator establishes that the host still exists; the exact host session, generation and transport remain current and open; shutdown has not started; operation identity and Moderate risk are unchanged; and a fresh fixed inventory still reports the private target as `loaded / failed / failed`. Renderer state and TTL alone are never sufficient. No remote compare-and-swap guarantee is implied.

The mutation transport contract has explicit dispatch certainty:

- `NotDispatched` identifies cancellation, timeout, connection failure or policy rejection known to occur before dispatch.
- `CompletionConfirmed` records a positively confirmed success or failure.
- `CompletionUnknown` records cancellation, timeout, connection loss or bounded-output overflow after dispatch occurred or may have occurred.

Internal outcomes are `Success`, `Failed`, `Cancelled` and `OutcomeUnknown`. `OutcomeUnknown` is terminal. It never triggers automatic retry, replay or authority restoration. A later attempt requires a fresh backend observation and plan. Confirmed failure does not claim the remote command had no partial side effects. There is no generic rollback: compensation is operation-specific and may not exist.

## Audit boundary

`AuditOutcome::OutcomeUnknown` extends the existing JSON enum without changing the persisted event fields, so legacy events containing `host`, `timestamp`, `operation`, `actor`, `outcome` and `durationMs` remain readable. Core now has an explicit native-risk recording path. Existing string-kind recording fails closed for unknown kinds instead of defaulting them to `ReadOnly`.

Audit remains metadata-only. It contains no command, executable, argv, environment, stdout, stderr, error text, credential, private authority payload or target name. Goal 05A performs no production mutation and therefore adds no synthetic plan audit events.

## Goal boundary

Goal 05B permits exactly three operation-specific IPC commands and display DTOs. Source policy retains each Rust source path and allows one native operation module plus one SSH adapter module while rejecting additional operations, adapters, generic command strings, argv/environment payloads and generic renderer mutation APIs elsewhere. The current `ServiceEntry.unit` display string is never accepted back as operational input. Real systemd mutation acceptance remains a separate owner-authorized gate.

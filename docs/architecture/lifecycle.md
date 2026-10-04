# Process lifecycle and quiescence

Goal 04F adds one desktop-native admission boundary for every custom Tauri command. The process begins in `Running`. A native caller can irreversibly seal it, after which no new renderer request is admitted. Commands that won the serialized admission race before sealing hold a counted RAII permit for their complete invocation, including every await. Sealing waits for those permits to leave and then records `Quiesced`; there is no transition back to `Running` in the same process.

The command drain uses one absolute 60-second deadline. A timeout returns a display-safe failure and leaves the process sealed. A later native attempt can wait for a previously admitted operation that has since completed, but it cannot reopen admission. Commands presented after sealing receive the same conflict error: `NexusOps is closing. Restart the application to continue.` The coordinator is native-only: it adds no renderer command, lifecycle state, capability permission or frontend action.

## Resource cleanup phases

Command quiescence and resource cleanup are deliberately separate primitives:

1. seal command admission and drain admitted commands;
2. shut down application resources through `Application::shutdown()`;
3. revoke all local picker grants;
4. explicitly finalize the non-blocking application log guard.

`Application::shutdown()` runs only after admission has drained. It atomically seals and revokes the Goal 05A remote-operation authority store, serializes a bounded connection-state transition, then releases the global metadata gate before resource I/O. It clears rotation authority, cancels connecting sessions, marks connected sessions for disconnect, permanently closes transfer admission, preserves owned staging cleanup, closes SFTP channels, revokes file plans and editor-document authority, clears SFTP startup gates, closes terminals, clears discovery gates, drops SSH transports and publishes disconnected session state. Repeating a successful shutdown is safe. The existing transfer cleanup deadline remains 60 seconds **per SFTP session**, so total resource shutdown can scale with the number of sessions. Goal 04F does not shorten or combine those conservative cleanup windows.

## Future remote operations

Goal 05A adds no custom command, but fixes how a future mutation will coexist with lifecycle admission. A renderer-callable mutation command must first hold an ordinary command permit. Native execution then tries a dedicated per-host operation gate followed by the process-wide one-operation semaphore; busy capacity returns immediately and never schedules work. A lifecycle edit, delete, disconnect or session replacement takes the same host gate before the short global metadata gate. It releases the metadata gate before transport cleanup or remote I/O. This ordering prevents `Application::mutation -> host gate` inversion and lets unrelated host metadata continue while Host A is busy.

Updater sealing before command admission rejects the future mutation. Updater sealing after command admission waits for that ordinary permit. Routine host mutation never invokes `seal_and_drain_others()`. Application shutdown seals and clears pending operation authority after command drain, so a plan cannot be recreated during teardown. See [remote-operation authority](remote-operations.md).

Local picker grants are revoked only after application resource shutdown succeeds. Revocation is idempotent and selected paths or handles do not cross the shutdown boundary. Logging is owned by a native `Mutex<Option<WorkerGuard>>`; finalization consumes the guard once and repeated finalization is safe. This provides explicit buffer-worker closure, but makes no durable `fsync` or secure-erasure claim.

## Normal exit and exclusive update installation

The first normal Tauri exit request is prevented while a single asynchronous attempt performs this order: command seal/drain, normal-exit `UpdateService::shutdown()`, `Application::shutdown()`, local-grant revocation and log finalization. Repeated exit requests during that attempt are prevented. After successful cleanup, native state is marked complete and the application requests exit again; that second `ExitRequested` event is allowed without another cleanup task.

If drain or resource cleanup fails, final exit is not requested. Admission remains sealed and logging remains available for safe stage and error-code metadata. A later explicit exit request retries cleanup without reopening normal work; there is no periodic retry. Logging never includes hostnames, paths, terminal or remote content, updater URLs, signatures, bytes, announcement IDs or credentials.

The reusable quiescence primitive intentionally does **not** shut down `UpdateService`. Normal exit adds that update cleanup as its own step. Goal 04G adds an install-exclusive transition that is serialized by the same mutex: the caller keeps its RAII permit, sealing succeeds only from `Running`, new admission stops, and the drain waits for `active == 1`. This avoids a self-deadlock without dropping the install permit. If normal exit seals first, installation fails before consumption. If installation seals first, a concurrent normal exit cannot reopen admission and may complete idempotent cleanup after the install permit leaves if the process is still alive. An exclusive timeout leaves the process sealed.

After exclusive drain, the update service atomically moves the exact retained installer context and verified bytes out of `Verified` and records `Installing`. Application shutdown, grant revocation and log finalization then precede the sole installer boundary. Update shutdown is deliberately absent from this chain because it would destroy the consumed authority. Installer preparation failures do not unseal the process. Automatic restart and relaunch remain disabled.

Source-policy tests enumerate every `#[tauri::command]`, require its injected lifecycle state and first-statement RAII admission, reject early permit release, and reject renderer lifecycle authority. They allow exactly one Windows-gated installer call in the dedicated module, require the opaque install command, exclusive drain, explicit no-restart/no-op hook policy and cleanup order, and reject any second call. Rust tests cover admission/seal/exclusive races, checked counter failure, paused-time drain timeout and retry, exit serialization/order/failure, repeated resource shutdown, grant revocation, update shutdown, log finalization and exact-byte installer invocation.

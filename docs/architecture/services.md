# Systemd service inventory and bounded service operations

Services observes loaded **system** service units for the current verified SSH connection. It does not enumerate installed-but-unloaded unit files or user services. The only new remote operation is the compile-time constant:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --all --type=service --property=Id --property=LoadState --property=ActiveState --property=SubState --property=CanReload --property=Description show
```

There are no renderer-provided command arguments, unit names, paths or remote filters. No sudo, polkit change or journal read is available. A hostile remote account can redefine commands or lie about results, so inventory is an observation rather than an attestation.

The service inventory uses selected systemd properties rather than parsing `systemctl`'s human-formatted `list-units` table, whose `JOB` column is dynamic. The `--type=service` filter makes `show` enumerate matching loaded units; `--all` includes inactive units and empty properties. Blank lines separate unit property blocks, and property order is irrelevant. The operation engine applies an 8-second deadline and 64 KiB output cap. The parser accepts at most 512 unique `.service` blocks. Unit names are at most 255 bytes; load/active/sub states at most 32 bytes each; descriptions at most 512 bytes and may be empty. Missing or duplicate properties, unexpected properties, duplicate units, controls, bidi formatting, ANSI escapes and malformed records fail the whole response with a fixed safe error. `CanReload` must occur exactly once and is accepted only as lowercase `yes` or `no`; it is published as display-only `canReload`. Future bounded state tokens remain visible without implying health. React renders validated fields only as text, without links or HTML.

Core requires the exact `HostId` and `HostSessionId`, captures the transport, cancellation token and generation, then revalidates all of them after SSH I/O. Requests use non-queuing per-host exclusion and a global limit of four. A successful snapshot atomically replaces a bounded memory-only actionable-observation set: one set per host, 120-second monotonic TTL, at most 512 rows per set, 64 host sets and 4096 operation capabilities globally. Strict units observed as `loaded / failed / failed` receive reset-failed IDs; strict units observed as `loaded / active / running` receive different try-restart IDs. Exact running rows with `CanReload=yes` also receive a separate reload ID, so restart and reload authority can coexist without a generic action selector. The backend binds every opaque ID to exactly one semantic operation. Cross-operation use fails closed. Per-host request sequences and lifecycle-gated publication prevent delayed older reads or old sessions from publishing authority. Disconnect/reconnect, remote closure, edit, delete and shutdown revoke all capability types. No service snapshot or observation authority enters SQLite, Query, Zustand, browser storage, audit or logs.

The Services component mounts only for the selected section. It requests an initial snapshot when connected and another only on explicit Refresh; there is no polling. It filters locally, drops delayed old-session responses and clears its state on session replacement/unmount. A successful empty list says “No loaded system services were returned.” An unsupported/non-systemd/inaccessible manager or malformed response says “Service inventory is unavailable on this host.” The SSH transport intentionally does not expose raw stderr, so the UI does not claim a specific permission denial. A failed Refresh can retain the last successful snapshot from the same session with a visible warning and timestamp.

Goal 05B adds exactly one structured mutation for an actionable row. Planning resolves the opaque observation ID, performs a fresh complete inventory, verifies the same strict unit is still `loaded / failed / failed`, then publishes a session/generation-bound one-shot plan under the host lifecycle gate. The review modal shows the backend-owned target, Moderate risk and the fixed effect, requires a separate checkbox, and discards pending authority on cancel, close, Escape, unmount or session turnover. The renderer never supplies a unit, verb, command, argv, environment or risk.

After one-shot consume, execution performs another fresh complete inventory while retaining the per-host service gate through dispatch. The only mapping is:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password reset-failed -- <STRICT_UNIT>
```

`STRICT_UNIT` is a native type using `PART["@"PART] ".service"`, where each nonempty part begins with ASCII alphanumeric and then contains only ASCII alphanumeric, `_`, `.`, `:` or `-`; the complete name is at most 255 bytes. Channel open is bounded to 5 seconds, request through confirmed completion to 15 seconds, cleanup to 2 seconds and combined stdout/stderr to 8 KiB. No sudo, retry, rollback or multi-unit form exists. Known exit status gives confirmed success/failure; cancellation, timeout or connection loss after possible dispatch yields terminal `OutcomeUnknown`. Every post-consume result attempts one metadata-only `service.reset_failed` / Moderate / User audit record and one separate fresh observation. Audit or observation failure cannot change known mutation truth.

Goal 05D adds one separate High-risk flow for an exact `loaded / active / running` row. It uses a different observation capability, plan/result DTOs and plan/discard/execute commands. The review names the backend-owned unit and exact state, explains temporary unavailability, dependency-related systemd jobs, the absence of application-health assurance and rollback, and requires an independent High-risk checkbox. The fixed SSH mapping is:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password try-restart -- <STRICT_UNIT>
```

Planning and post-consume execution each revalidate through a fresh full inventory. The 5-second channel-open, 30-second completion, 2-second cleanup and 8 KiB aggregate-output limits are fixed. `WindowAdjusted` and EOF are non-terminal; exit status or exit signal supplies terminal evidence. Close, timeout, cancellation, session loss or output overflow after possible dispatch yields terminal `OutcomeUnknown`, with no retry. `try-restart` does not start a unit that has become inactive, but a concurrent remote actor can still race observation and dispatch. Every consumed authority attempts one `service.try_restart` / High / User metadata-only audit and one separate best-effort observation. Successful systemctl completion is not an application-health claim.

Goal 05E is a source-review candidate for one further operation-specific High-risk flow. A reload capability exists only for an exact `loaded / active / running / CanReload=yes` row. Planning and execution each repeat the complete inventory and exact capability check. The renderer supplies only the opaque reload observation ID; it never supplies the unit, state, verb, command, risk or timeout. The fixed mapping is:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password reload -- <STRICT_UNIT>
```

Reload has its own plan/discard/execute IPC family, one-shot approval and independent checkbox. The transport retains the 5-second open, 30-second completion, 2-second cleanup and 8 KiB aggregate-output bounds. It has no retry, rollback, restart fallback, sudo or generic service verb. Every consumed authority attempts exactly one metadata-only `service.reload` / High / User audit and one separate best-effort service observation. Systemctl success is not a claim about configuration validity, health or availability.

## Validation and native acceptance

Goal 05B real-systemd acceptance and Goal 05C Native Windows desktop acceptance passed on the merged implementation. Goal 05D passed deterministic, unit and source-policy validation; real OpenSSH systemctl-absent interoperability; disposable Ubuntu 24.04 real-systemd acceptance; and Native Windows desktop acceptance. The accepted production path was:

```text
Windows desktop UI
→ operation-specific Tauri IPC
→ Application
→ one-shot native authority
→ SshSession
→ SystemdTryRestart transport
→ real OpenSSH
→ real systemd
```

Acceptance used an exact `loaded / active / running` target and one High-risk confirmation. It produced exactly one try-restart dispatch; the service remained `loaded / active / running`; MainPID and InvocationID changed while `NRestarts=0`; SSH remained usable; exactly one metadata-only success audit was recorded; and no automatic retry occurred.


Goal 05E real-systemd reload and Native Windows desktop acceptance remain pending and require separate owner authorization after exact-head source review. Implementation validation may use only the established systemctl-absent, non-mutating OpenSSH interoperability path.

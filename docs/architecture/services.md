# Systemd service inventory and bounded service operations

Services observes loaded **system** service units for the current verified SSH connection. It does not enumerate installed-but-unloaded unit files or user services. The base inventory remote operation is the compile-time constant:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --all --type=service --property=Id --property=LoadState --property=ActiveState --property=SubState --property=CanStart --property=CanReload --property=Description show
```

There are no renderer-provided command arguments, unit names, paths or remote filters. No sudo, polkit change or journal read is available. A hostile remote account can redefine commands or lie about results, so inventory is an observation rather than an attestation.

The service inventory uses selected systemd properties rather than parsing `systemctl`'s human-formatted `list-units` table, whose `JOB` column is dynamic. The `--type=service` filter makes `show` enumerate matching loaded units; `--all` includes inactive units and empty properties. Blank lines separate unit property blocks, and property order is irrelevant. The operation engine applies an 8-second deadline and 64 KiB output cap. The parser accepts at most 512 unique `.service` blocks. Unit names are at most 255 bytes; load/active/sub states at most 32 bytes each; descriptions at most 512 bytes and may be empty. Missing or duplicate properties, unexpected properties, duplicate units, controls, bidi formatting, ANSI escapes and malformed records fail the whole response with a fixed safe error. `CanStart` and `CanReload` must each occur exactly once and accept only lowercase `yes` or `no`; they are published as display-only `canStart` and `canReload`. Future bounded state tokens remain visible without implying health. React renders validated fields only as text, without links or HTML.

Core requires the exact `HostId` and `HostSessionId`, captures the transport, cancellation token and generation, then revalidates all of them after SSH I/O. Requests use non-queuing per-host exclusion and a global limit of four. A successful snapshot atomically replaces a bounded memory-only actionable-observation set: one set per host, 120-second monotonic TTL, at most 512 rows per set, 64 host sets and 4096 operation capabilities globally. Strict units observed as `loaded / failed / failed` receive reset-failed IDs; strict units observed as `loaded / active / running` receive different try-restart IDs. Exact running rows with `CanReload=yes` also receive a separate reload ID. Exact `loaded / inactive / dead / CanStart=yes` rows receive a separate start ID. Running, failed, activating, deactivating, inactive/exited, not-found or `CanStart=no` rows receive no Start authority. The backend binds every opaque ID to exactly one semantic operation. Cross-operation use fails closed. Per-host request sequences and lifecycle-gated publication prevent delayed older reads or old sessions from publishing authority. Disconnect/reconnect, remote closure, edit, delete and shutdown revoke all capability types. No service snapshot or observation authority enters SQLite, Query, Zustand, browser storage, audit or logs.

The Services component mounts only for the selected section. It requests an initial snapshot when connected and another only on explicit Refresh; there is no polling. It filters locally, drops delayed old-session responses and clears its state on session replacement/unmount. A successful empty list says “No loaded system services were returned.” An unsupported/non-systemd/inaccessible manager or malformed response says “Service inventory is unavailable on this host.” The SSH transport intentionally does not expose raw stderr, so the UI does not claim a specific permission denial. A failed Refresh can retain the last successful snapshot from the same session with a visible warning and timestamp.

## Goal 05G bounded read-only stop-impact assessment

Each validated inventory row receives a separate opaque, memory-only stop-impact inspection ID. It is bound to the host, SSH session, connection generation, inventory request sequence and a 120-second monotonic TTL. The renderer can return only that ID with the exact host/session identity; it cannot submit a unit, relationship, systemctl property, command, plan or mutation target. Disconnect, reconnect, remote closure, host edit/delete and shutdown revoke inspection IDs. Inspection results remain component-local and are never persisted, audited or converted into remote-operation authority.

The backend resolves the unit and constructs only this native-owned query, batching at most 16 validated unit identities after `--`:

```text
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password --all --property=Id,Names,Following,LoadState,ActiveState,SubState,CanStop,RefuseManualStop,Job,NeedDaemonReload,StopWhenUnneeded,Requires,RequiredBy,Requisite,RequisiteOf,Wants,WantedBy,BindsTo,BoundBy,PartOf,ConsistsOf,PropagatesStopTo,StopPropagatedFrom,Upholds,UpheldBy,Conflicts,ConflictedBy,Before,After,Triggers,TriggeredBy,OnSuccess,OnFailure,OnSuccessJobMode,OnFailureJobMode,SuccessAction,FailureAction show -- '<UNIT>'...
```

The fixed contract contains exactly 37 properties. Every property must occur exactly once in each blank-line-separated block; duplicate, missing, unknown, oversized or malformed data fails closed. `Job` is normalized only from empty or an exact decimal `NonZeroU32`, and no D-Bus object path is published. Unsupported bounded job-mode or manager-action values remain diagnostic data but make impact unknown. Canonical `Id`, `Names` and `Following` resolve aliases before non-root query deduplication. Each multi-unit response is matched by validated `Id`/`Names`, independently of response order; missing, extra, duplicate or ambiguous identities reject the batch. Unresolved alias collisions make the result partial and uncertain. Root is the only unit that may be queried twice: initial observation and the final actual root consistency query.

List properties normalize one CLI quoting layer before strict typed unit validation and duplicate detection. The encoding is pinned to systemd v255 commit `db11bab38ccf1ed257f310d29070843d4c58ea01`: its [string-array printer](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/src/shared/bus-print-properties.c#L228) calls `shell_maybe_quote(str, 0)`, whose [default encoder](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/src/basic/escape.c#L474) wraps special identities in double quotes and doubles literal backslashes. The parser removes those CLI escapes once, preserving canonical `\xHH` identity text literally, including hex spelling. Existing literal unquoted and single-quoted identities remain supported. Unterminated or concatenated quotes, unsupported escapes, control bytes, unsafe decoded unit text, duplicate normalized identities and ambiguous multi-unit matches fail closed. Encoded property bounds apply before normalization; normalization never expands input or adds an auxiliary collection. Synthetic regressions cover the acceptance failure's valid service/slice candidate batch without retaining host output.

An assessment performs at most six actual SSH queries with no retry: one root query, zero to four nonempty traversal batches and one final root revalidation. The final query slot and its eight-second window remain reserved throughout the 20-second total deadline. No dummy request is emitted and nothing is queried after final revalidation. Direct frontier work always precedes conditional probes, including when a later direct edge upgrades an already queued conditional identity. Within each frontier class, work is stable by depth, canonical-or-observed unit name and relationship type, so input property ordering cannot choose which nodes fit inside an exhausted query budget. A failed or malformed traversal batch stops traversal and proceeds directly to the reserved root revalidation; a malformed final root produces a partial, unknown result. Each request has an eight-second deadline and 64 KiB response limit; Stop-impact rejects a batch if any stderr is present while keeping its bounded contents private. Property values are at most 8 KiB, blocks 32 KiB and aggregate remote output 256 KiB. Retained topology is bounded to 64 nodes, 512 edges, depth four, 16 aliases per unit and 2 MiB of conservatively accounted in-memory working state.

Diagnostic projection reserves its storage within that same 2 MiB accounting limit before traversal. It streams borrowed diagnostic views into a sorted vector of at most 512 owned entries, evicting before insertion, and deduplicates normalized related identities using a fixed-capacity borrowed vector reset between properties and canonical records. Every unique diagnostic is counted even after retention fills, so omitted-known diagnostic counts remain exact without materializing all diagnostics or changing graph-edge accounting; diagnostic truncation reports `Partial` and `UnknownImpact`.

Only `RequiredBy`, `BoundBy`, `ConsistsOf` and `PropagatesStopTo` propagate direct stop impact recursively. `Requires`, `Wants`, `Requisite`, `BindsTo` and `Upholds` admit bounded `StopWhenUnneeded` candidates. A candidate is retained conditionally only when `StopWhenUnneeded=yes` and every observed reverse reference is already in the affected set. An observed unaffected reverse reference is classified explicitly; an unresolved or ambiguous reverse reference produces incomplete coverage and unknown impact rather than silently eliminating the candidate. Direct discovery upgrades a cached conditional node and its cached descendants in memory; it never causes a duplicate SSH query. Ordering-only, trigger, conflict, success/failure action, pending-job and daemon-reload data can increase uncertainty but cannot become direct stop propagation.

The DTO separates technical completeness (`complete`, `partial`, `unavailable`) from conditional or unknown impact. It publishes validated retained relationship edges, classified conditional consequences and bounded typed passive context for observed success/failure activation, trigger relationships, `UpheldBy` reactivation context and manager-action consequences. It never publishes executable command definitions or environment data, and an observed reverse relationship is not presented as proof that a unit remains active or will reactivate. Unsupported bounded job-mode or manager-action enumerants produce explicit diagnostic evidence, `partial` completeness and `unknownImpact`; they are never reinterpreted. Graph-edge accounting and passive-diagnostic accounting have separate retained and known-omission counters. The 512-item diagnostic limit cannot increase omitted graph-edge counts; omitting known diagnostic context produces `partial` completeness and `unknownImpact`. Accounting otherwise distinguishes successfully observed records/references, retained nodes/edges/candidates, known parsed omissions and exact unresolved frontier references; it never estimates undiscovered topology. A complete result means only that this bounded systemd graph query completed within its contract. It does not mean stopping is safe. Unknown application dependencies, service stop hooks, process ownership, SSH/VPN/network reachability and concurrent systemd changes remain outside the systemd-managed graph. Starting Refresh invalidates any visible or in-flight assessment for that service snapshot, and no new assessment starts while refresh is pending. The UI states these limits and provides no Stop approval or execution control.

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

Goal 05E adds the completed third operation-specific High-risk flow. A reload capability exists only for an exact `loaded / active / running / CanReload=yes` row. Planning and execution each repeat the complete inventory and exact capability check. The renderer supplies only the opaque reload observation ID; it never supplies the unit, state, verb, command, risk or timeout. The fixed mapping is:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password reload -- <STRICT_UNIT>
```

Reload has its own plan/discard/execute IPC family, one-shot approval and independent checkbox. The transport retains the 5-second open, 30-second completion, 2-second cleanup and 8 KiB aggregate-output bounds. It has no retry, rollback, restart fallback, sudo or generic service verb. Every consumed authority attempts exactly one metadata-only `service.reload` / High / User audit and one separate best-effort service observation. Systemctl success is not a claim about configuration validity, health or availability.

## Goal 05F bounded systemd Start

Start is a fourth, separate High-risk operation. Its capability exists only for an exact `loaded / inactive / dead / CanStart=yes` row. Planning and execution repeat the complete inventory and exact check; the renderer supplies only the opaque Start observation ID. The fixed mapping is:

```text
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password start -- <STRICT_UNIT>
```

Start has its own plan/discard/execute IPC family and independent confirmation checkbox. It uses the 5-second open, 30-second completion, 2-second cleanup and 8 KiB aggregate-output bounds. There is no retry, rollback, Stop, `--no-block`, sudo or permission fallback. `CanStart=yes` does not imply that the SSH account is authorized or that startup is safe, healthy or lasting. Systemd may activate dependencies, bind network listeners or process queued work. Every consumed authority attempts exactly one metadata-only `service.start` / High / User audit and one best-effort fresh service observation. A successful Start command completion does not prove application health, configuration validity, dependency health, continued running or exclusive NexusOps causation.

Goal 05F passed source review, source-policy re-review, OpenSSH systemctl-absent interoperability, disposable real-systemd Start acceptance and Native Windows desktop end-to-end acceptance. The implementation HEAD `90c21099318d787c49cb0b627ca46707b81aeb06` was merged as `4548d23d88d1729b4ae41c917f50ed8d2227b0e3`, and the post-merge push workflow passed on Ubuntu, Windows and macOS. The accepted production path was:

```text
Windows production desktop UI
→ operation-specific Tauri IPC
→ Application
→ one-shot SystemdStart authority
→ SshSession
→ typed SystemdStart transport
→ real OpenSSH
→ real systemd
```

The eligible target exposed Start while a non-startable control did not. The High-risk confirmation gate required its independent checkbox. Exactly one Start dispatch changed `ExecStart` from `0 → 1` and MainPID from `0 → >0`, producing `loaded / active / running` with `NRestarts=0`; SSH and the product session survived. Manual Refresh removed Start and exposed the existing Try-Restart capability for the now-running service. Exactly one metadata-only success audit was persisted, with no retry or Stop. Disposable real-systemd acceptance explicitly proved same-plan replay rejection. The Windows UI gate had no established replay instrumentation and did not manually replay the consumed plan; it instead proved one Start click and dispatch, no second Start, capability removal after Refresh and an `ExecStart` count that remained exactly `1`.

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


Goal 05E passed deterministic, unit and source-policy validation; real OpenSSH systemctl-absent interoperability; disposable Ubuntu 24.04 real-systemd reload acceptance; Native Windows desktop end-to-end acceptance; merge; and post-merge CI. The accepted production path was:

```text
Windows desktop UI
→ operation-specific Tauri IPC
→ Application
→ one-shot SystemdReload authority
→ SshSession
→ typed SystemdReload transport
→ real OpenSSH
→ real systemd
```

Acceptance used an exact `loaded / active / running / CanReload=yes` target, while a running non-reloadable control received no reload capability and Reload remained distinct from Try-Restart. It produced exactly one reload dispatch and changed the fixture `ExecReload` counter from `0 → 1`; MainPID, InvocationID and the process start timestamp remained unchanged with `NRestarts=0`, and the service and SSH session remained usable. Exactly one metadata-only success audit was recorded, with no automatic retry or restart fallback.

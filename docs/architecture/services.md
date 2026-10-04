# Systemd service inventory and bounded reset-failed operation

Services observes loaded **system** service units for the current verified SSH connection. It does not enumerate installed-but-unloaded unit files or user services. The only new remote operation is the compile-time constant:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --all --type=service --property=Id --property=LoadState --property=ActiveState --property=SubState --property=Description show
```

There are no renderer-provided command arguments, unit names, paths or remote filters. No sudo, polkit change or journal read is available. A hostile remote account can redefine commands or lie about results, so inventory is an observation rather than an attestation.

The service inventory uses selected systemd properties rather than parsing `systemctl`'s human-formatted `list-units` table, whose `JOB` column is dynamic. The `--type=service` filter makes `show` enumerate matching loaded units; `--all` includes inactive units and empty properties. Blank lines separate unit property blocks, and property order is irrelevant. The operation engine applies an 8-second deadline and 64 KiB output cap. The parser accepts at most 512 unique `.service` blocks. Unit names are at most 255 bytes; load/active/sub states at most 32 bytes each; descriptions at most 512 bytes and may be empty. Missing or duplicate properties, unexpected properties, duplicate units, controls, bidi formatting, ANSI escapes and malformed records fail the whole response with a fixed safe error. Future bounded state tokens remain visible without implying health. React renders validated fields only as text, without links or HTML.

Core requires the exact `HostId` and `HostSessionId`, captures the transport, cancellation token and generation, then revalidates all of them after SSH I/O. Requests use non-queuing per-host exclusion and a global limit of four. A successful snapshot replaces a bounded memory-only actionable-observation set: one set per host, 120-second monotonic TTL, at most 512 rows/actionable IDs per set, 64 host sets and 4096 actionable IDs globally. Only strict operational unit names observed as `loaded / failed / failed` receive opaque IDs. Per-host request sequences and lifecycle-gated publication prevent delayed older reads or old sessions from publishing authority. Disconnect/reconnect, remote closure, edit, delete and shutdown revoke observations. No service snapshot or observation authority enters SQLite, Query, Zustand, browser storage, audit or logs.

The Services component mounts only for the selected section. It requests an initial snapshot when connected and another only on explicit Refresh; there is no polling. It filters locally, drops delayed old-session responses and clears its state on session replacement/unmount. A successful empty list says “No loaded system services were returned.” An unsupported/non-systemd/inaccessible manager or malformed response says “Service inventory is unavailable on this host.” The SSH transport intentionally does not expose raw stderr, so the UI does not claim a specific permission denial. A failed Refresh can retain the last successful snapshot from the same session with a visible warning and timestamp.

Goal 05B adds exactly one structured mutation for an actionable row. Planning resolves the opaque observation ID, performs a fresh complete inventory, verifies the same strict unit is still `loaded / failed / failed`, then publishes a session/generation-bound one-shot plan under the host lifecycle gate. The review modal shows the backend-owned target, Moderate risk and the fixed effect, requires a separate checkbox, and discards pending authority on cancel, close, Escape, unmount or session turnover. The renderer never supplies a unit, verb, command, argv, environment or risk.

After one-shot consume, execution performs another fresh complete inventory while retaining the per-host service gate through dispatch. The only mapping is:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-ask-password reset-failed -- <STRICT_UNIT>
```

`STRICT_UNIT` is a native type using `PART["@"PART] ".service"`, where each nonempty part begins with ASCII alphanumeric and then contains only ASCII alphanumeric, `_`, `.`, `:` or `-`; the complete name is at most 255 bytes. Channel open is bounded to 5 seconds, request through confirmed completion to 15 seconds, cleanup to 2 seconds and combined stdout/stderr to 8 KiB. No sudo, retry, rollback or multi-unit form exists. Known exit status gives confirmed success/failure; cancellation, timeout or connection loss after possible dispatch yields terminal `OutcomeUnknown`. Every post-consume result attempts one metadata-only `service.reset_failed` / Moderate / User audit record and one separate fresh observation. Audit or observation failure cannot change known mutation truth.

## Validation and native acceptance

Deterministic parser, policy, core lifecycle, loopback SSH and component tests cover the Goal 05B source candidate. Real systemd mutation and Native Windows acceptance are not part of this implementation task and remain pending separate owner authorization. The earlier read-only inventory acceptance passed for source-reviewed HEAD `354598dc7a527c129172c0834db7e2113e7a6eb9`; it is not evidence for the new mutation.

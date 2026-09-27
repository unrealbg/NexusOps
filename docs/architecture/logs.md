# System journal snapshots

Logs is a bounded read-only view of potentially sensitive system journal content for the selected connected Linux host. Opening the workspace makes one request; manual Refresh makes one more. There is no journal polling, focus refetch, follow stream, pagination or historical storage. Containers remains reserved.

## Authority and bounds

`logsApi.list(HostId, HostSessionId)` calls the single `list_host_logs` Tauri command and `Application::list_host_logs`. The renderer cannot provide command arguments, paths, units, priorities, search text, time ranges, boots, cursors or limits. The only new `ReadOnlyCommand` is `SystemJournal`, operation kind `logs.list`, risk `ReadOnly`, with exactly:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 journalctl --system --no-pager --quiet --boot=0 --reverse --lines=10 --output=json --output-fields=MESSAGE,PRIORITY,_SYSTEMD_UNIT,SYSLOG_IDENTIFIER
```

This requests the newest ten accessible system journal records from the current boot. It uses only the SSH account's existing permissions. No sudo, polkit, group/ACL changes, packages, journal configuration, rotation, vacuum, flush, sync, file reads or other remote mutation is added. There is no `--all` or follow mode. A compromised remote account can redefine its utilities; the fixed command expresses requested authority, not attestation of a hostile server.

The existing operation engine limits execution to eight seconds and output to 64 KiB; the SSH transport's combined stdout/stderr and channel protections remain unchanged. Invalid operation kind or write risk is rejected before execution. The parser independently bounds raw input and counts at most ten JSON-lines records; overflow and malformed structures cannot produce partial success.

## Projection and display safety

The parser lives in `nexus-discovery`. It follows the [systemd Journal JSON format](https://systemd.io/JOURNAL_EXPORT_FORMATS/#journal-json-format): fields may be text, null, binary byte arrays or arrays representing multiple field values. Temporary raw records never become DTOs. Unknown fields are ignored; `__CURSOR`, `__MONOTONIC_TIMESTAMP` and `_BOOT_ID` are neither published nor persisted. Cursor uniqueness is not required.

- Each line must be an object. Empty output is valid; a trailing newline is allowed; blank records and duplicate selected JSON keys are rejected. Journal multi-value arrays are distinct from duplicate JSON keys.
- `__REALTIME_TIMESTAMP` must be one nonempty decimal text value representing nonnegative Unix microseconds. Parsing is checked for overflow and the supported UTC year range through 9999. Publication uses canonical UTC RFC3339 with six fractional digits, never the raw timestamp. Null, arrays, numbers, malformed text and missing timestamps fail closed.
- `MESSAGE` text is limited to 4096 UTF-8 bytes; larger raw text fails closed. Missing MESSAGE becomes `Missing`. Null, binary/multi-value arrays and all other non-text JSON values become `Omitted`, never arbitrary JSON text.
- Display normalization escapes backslash as `\\`, line feed/carriage return/tab as `\n`/`\r`/`\t`, and prohibited controls as visible lowercase `\u{hhhh}` escapes (at least four hex digits). It covers C0/C1, ESC, bidi marks/overrides/isolates, zero-width and format controls, BOM, line/paragraph separators, variation selectors and tag controls. No raw prohibited controls remain. If expansion exceeds 8192 bytes the whole message becomes `Omitted`; it is never silently truncated.
- `Text` has a present message, including a valid empty string. `Missing` and `Omitted` have no message. The UI labels these states separately without fabricating content.
- `_SYSTEMD_UNIT` is a nonempty display-safe token of at most 255 UTF-8 bytes. `SYSLOG_IDENTIFIER` is nonempty display-safe text of at most 128 bytes; ordinary spaces are allowed. Unsafe controls or oversized text fail closed. Missing/null/array/object values become absent. Other scalar types fail closed.
- PRIORITY accepts only the strings `0` through `7`, projected to a typed Emergency–Debug enum. Missing/null/array/object values become absent. Invalid scalar values fail closed.

The renderer uses text nodes without HTML, ANSI interpretation, linkification, click-to-open, Copy or Export. A local case-insensitive filter searches only the current snapshot and makes no remote request. Priority is a factual label, not a health, severity-of-host or compliance verdict.

## Ownership and lifetime

Core verifies repository ownership, exact Connected state and `HostSessionId`, captures the transport, cancellation token and generation, then releases the metadata gate before I/O. Before returning it rechecks cancellation, transport closure, generation, state and both session identities. Disconnect/reconnect, deletion or edit cannot publish old authority.

Independent `log_gates` allow one active read per host and `log_limit` permits four globally. Admission is non-queuing: duplicate/capacity conflicts return a fixed safe error. Logs consumes no Services or Network permit. Delete removes its per-host gate; shutdown clears all Logs gates. Timeout/cancellation drops permits for later requests.

Existing `useHostSession` supplies connection state with its unchanged polling lifetime. A keyed connected child owns snapshot, errors, filter and request generation in React memory. Disconnect removes the child; host/session replacement remounts it. Cleanup invalidates in-flight continuations. Mismatched DTO identities never publish. Refresh disables overlapping clicks and has an immediate in-flight guard. A failed same-session refresh can retain the prior timestamped snapshot with: “Refresh failed — showing the last successful snapshot.” A new session cannot retain it.

An empty successful read means only “No accessible system journal entries were returned.” It does not mean the host has no logs. Unavailable journalctl/system journal, access denial, malformed data and execution failure produce a fixed unavailable message, never stderr or journal content. No access escalation is attempted.

## Sensitive content and validation

Messages may contain secrets, tokens, URLs, commands, paths and user payloads. This is a more sensitive read capability than topology/service metadata; there is no content-redaction guarantee. Transient SSH buffers, Rust parser/DTO data, IPC and active component memory necessarily contain the selected content. Journal fields, timestamps and filter text never enter application logs, audit, SQLite, Query, Zustand, browser storage, history, screenshots or published evidence. The read adds no audit event or persistence. No new clipboard/filesystem permissions or dependencies are needed. Process memory, a compromised renderer/local account and OS/browser internals remain outside a secure-erasure guarantee.

Deterministic parser tests cover JSON variants, projection, timestamp conversion, raw/display bounds and inert control normalization. Core tests exercise session turnover, deletion, closure, admission isolation, cleanup and timeout recovery. Component tests cover initial/manual-only requests, 65-second idle/focus behavior, safe errors, stale-result isolation, optional fields, inert display and absent Copy/Export. The real SSH protocol loopback uses synthetic JSON-lines and tests malformed, oversized and unavailable responses without invoking a host shell.

Real current-boot journalctl interoperability and Native Windows acceptance remain **PENDING SOURCE REVIEW / NATIVE ACCEPTANCE**. Source tests and compilation are not native acceptance. After separate exact-head review, acceptance requires an immutable executable, isolated application and WebView profiles, a disposable systemd/OpenSSH fixture, invocation accounting and synthetic privacy/control canaries. No production VPS or real secrets are test data.

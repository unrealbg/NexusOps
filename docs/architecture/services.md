# Read-only systemd service inventory

Services observes loaded **system** service units for the current verified SSH connection. It does not enumerate installed-but-unloaded unit files or user services. The only new remote operation is the compile-time constant:

```sh
LC_ALL=C SYSTEMD_COLORS=0 SYSTEMD_URLIFY=0 systemctl --system --no-pager --no-legend --plain --full --all --type=service --no-ask-password list-units
```

There are no renderer-provided command arguments, unit names, paths or remote filters. No sudo, polkit change, service mutation or journal read is available. A hostile remote account can redefine commands or lie about results, so this is an observation rather than an attestation.

The operation engine applies an 8-second deadline and 64 KiB output cap. The parser accepts at most 512 unique `.service` rows. Unit names are at most 255 bytes; load/active/sub states at most 32 bytes each; descriptions at most 512 bytes. Missing fields, duplicates, controls, bidi formatting, ANSI escapes and malformed records fail the whole response with a fixed safe error. Future bounded state tokens remain visible without implying health. React renders validated fields only as text, without links or HTML.

Core requires the exact `HostId` and `HostSessionId`, captures the transport, cancellation token and generation, then revalidates all of them after SSH I/O. Requests use non-queuing per-host exclusion and a global limit of four. Disconnect/reconnect, remote closure, edit, delete and shutdown invalidate in-flight work. The application retains only admission gates, not service data; host deletion and shutdown clear gate bookkeeping. No service data enters SQLite, Query, Zustand, browser storage, audit or logs.

The Services component mounts only for the selected section. It requests an initial snapshot when connected and another only on explicit Refresh; there is no polling. It filters locally, drops delayed old-session responses and clears its state on session replacement/unmount. A successful empty list says “No loaded system services were returned.” An unsupported/non-systemd/inaccessible manager or malformed response says “Service inventory is unavailable on this host.” The SSH transport intentionally does not expose raw stderr, so the UI does not claim a specific permission denial. A failed Refresh can retain the last successful snapshot from the same session with a visible warning and timestamp.

## Validation and native acceptance

Deterministic parser, policy, core lifecycle and component tests cover bounds, identity, concurrency, cancellation and UI states. Production SSH interop on a real systemd target remains a separate acceptance gate when such a safe target is available. Native review should verify service rows and states without sudo, refresh/filter, empty and unsupported states, disconnect/reconnect and host switching, and that Terminal and Files remain usable. No service start/stop or remote mutation should be performed for acceptance. The Alpine/OpenSSH fixture is a useful non-systemd negative case; it cannot prove real systemd interoperability.

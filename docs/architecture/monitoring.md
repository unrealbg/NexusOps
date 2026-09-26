# Live host monitoring

Goal 03A samples a connected Linux host through the existing verified SSH transport. It installs no agent and exposes no generic command API. The operation allowlist contains fixed reads for `/proc/stat`, `/proc/meminfo`, `/proc/net/dev`, and the existing `LC_ALL=C df -Pk /` root-filesystem probe. Remote output is bounded to 64 KiB per command, parsed as untrusted data, and never written to application or audit logs.

## Sampling and derivation

The Overview requests one sample immediately and then five seconds after each request completes. This completion-based timer prevents overlapping request storms when a host is slow. Polling stops when Overview is hidden, the host disconnects, or the component unmounts. Rust serializes observations per host; there is no independent background monitor.

The aggregate `cpu ` row from `/proc/stat` supplies CPU counters. NexusOps sums `user`, `nice`, `system`, `idle`, `iowait`, `irq`, `softirq`, and `steal`; Linux already includes guest time in user/nice, so `guest` and `guest_nice` are validated but not added again. Between two observations, utilization is `(total delta - idle delta) / total delta * 100`, where idle includes `idle + iowait`. A first observation, zero total delta, changed field shape, decrease, or overflow resets the baseline and returns no percentage.

RAM continues to use `MemAvailable`, with the documented older-kernel approximation from free, buffers, cache, reclaimable slab, and shared memory. Swap requires distinct `SwapTotal` and `SwapFree` fields; a real zero total means “Not configured.” Root-disk values reuse the bounded POSIX `df` parser.

Network sampling parses at most 128 unique interface rows from `/proc/net/dev`. Names are at most 64 bytes and cannot contain whitespace or control characters. Each row must contain exactly the 16 Linux counters. Aggregate RX/TX excludes `lo` and uses checked sums. Rates divide checked byte deltas by monotonic elapsed time. A first observation, changed interface set, decreased counter, invalid elapsed interval, or overflow resets the aggregate network baseline instead of producing a negative or invented rate.

Individual parser or command failures become metric-specific warnings. A failed network read does not hide valid memory data. A transport/session failure rejects the observation. Missing values remain unavailable; valid zero rates remain zero.

## Session and frontend lifecycle

Every request and response carries exact `HostId + HostSessionId` ownership. Core captures the connection generation before remote work and revalidates generation, cancellation, connection state, and session identity before it commits a baseline. Disconnect, reconnect, host edit, delete, remote closure, and shutdown invalidate baselines. CPU and network deltas are never calculated across SSH sessions or hosts.

The renderer checks the returned ownership again. It ignores late completions after cleanup and filters displayed samples by current host/session identity. A reconnect starts an empty chart. Each chart retains at most 120 samples, approximately ten minutes at the default cadence, in component-local memory. No samples or histories are stored in SQLite, localStorage, sessionStorage, IndexedDB, Zustand, audit records, or application logs.

Freshness uses the sample UTC timestamp. The UI shows warming until the first response, marks a sample stale after 15 seconds, and distinguishes disconnected, unavailable, warming, valid zero, and no configured swap. SVG charts keep gaps for missing values and are decorative; current textual values remain accessible.

## Limits

Monitoring currently supports Linux hosts exposing the expected procfs records and a POSIX-compatible `df -P`. It monitors aggregate CPU, RAM, swap, the root filesystem, and aggregate non-loopback network bytes only. It does not inspect payloads, sockets, processes, services, containers, logs, other mounts, or persistent telemetry, and it does not alert.

## Native Windows acceptance

Native Windows acceptance passed on a production Tauri executable built from source-reviewed HEAD `ab02ecc649d3774fd1d29d5d298188678d17456c`. The [PR #4 acceptance evidence](https://github.com/unrealbg/NexusOps/pull/4#issuecomment-5844628069) records automatic monitoring, first-sample warm-up, subsequent CPU/network values, plausible memory/swap/root-disk values, valid `0 B/s`, responsive Terminal and Files workspaces, disconnect/reconnect and remote-closure behavior, and an 11-minute-plus soak. The 120-sample history cap is covered by a deterministic component regression.

Multi-host isolation and forced sampling delays longer than 15 seconds were **NOT EXECUTED** in native Windows acceptance because the environment lacked a second safe Linux host and a safe deterministic delay mechanism.

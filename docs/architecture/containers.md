# Local Docker container inventory

Containers is an on-demand, read-only inventory for the selected connected Linux host. It reads only the standard Docker Engine system daemon through `unix:///var/run/docker.sock`. It does not support Podman, rootless or custom sockets, remote Docker endpoints, Docker contexts, Compose or Swarm. If the authenticated SSH account cannot access the system socket, the inventory is unavailable. Docker daemon access can be highly privileged; NexusOps never grants it or changes socket permissions, groups, daemon configuration or packages. It uses no sudo or other elevation.

## Fixed authority and bounds

The renderer passes only `HostId` and `HostSessionId` to `list_host_containers`. Rust selects the sole `ReadOnlyCommand::DockerContainers` operation, kind `containers.list`, with this exact command:

```sh
LC_ALL=C docker --host unix:///var/run/docker.sock container ls --last 64 --no-trunc --format '{"id":{{json .ID}},"image":{{json .Image}},"name":{{json .Names}},"state":{{json .State}},"status":{{json .Status}},"ports":{{json .Ports}},"networks":{{json .Networks}}}'
```

The explicit `--host` pins the requested endpoint to the SSH host's local system socket; the renderer cannot choose a provider, endpoint, context, command, container ID, filter, state, limit or format. This is one Docker remote operation type, `container ls`. It requests up to the 64 most recently created containers in all states. There is no pagination or second read for overflow. No Docker inspect, logs, stats, events, actions, API forwarding, image management or mutation is exposed.

The unchanged operation engine bounds execution to eight seconds and output to 64 KiB; the SSH transport retains its combined stdout/stderr bound. The discovery parser independently rejects more than 64 JSON-lines records, malformed output, duplicate/missing/unknown fields, invalid full IDs, unknown states, oversized fields and deceptive display controls. It publishes only the seven selected fields, with typed state, or fails the whole snapshot. Empty successful output means only that this invocation returned no containers. An unavailable CLI, socket, daemon, permission, timeout or invalid response produces a fixed safe unavailable message without remote diagnostics.

## Session and display lifecycle

Core requires an existing connected host and exact session identity before I/O and revalidates transport, cancellation and generation afterwards. Separate non-queuing Containers admission permits one request per host and four globally, without consuming Services, Network or Logs permits. Disconnect, reconnect, host edit/deletion and remote closure cannot publish an old result.

The workspace requests once on entry with a valid session and once per manual Refresh. It does not poll or refetch on focus. Its snapshot and local filter remain in component memory only; host/session replacement or session-query failure removes them. A failed same-session Refresh retains the timestamped last successful snapshot with a warning. Rows are inert text without links, HTML, Copy, Export or container actions. No container inventory is persisted in SQLite, browser storage, Query/Zustand state, logs or audit payloads. Selected metadata can still reveal private image names, networks and published ports; display validation is not secret redaction. The active renderer and process memory necessarily hold the snapshot temporarily.

The remote account can replace or redefine `docker` in its shell environment, and a hostile server can return false data. This operation expresses bounded requested authority; it is not attestation that the host or daemon is trustworthy. Real Docker system-socket interoperability remains for native acceptance after exact-head source review.

# Manual update check and verified download

Goal 04D introduced the explicit application-global availability check. Goal 04E adds a second explicit action that downloads the currently announced updater artifact, verifies its Minisign signature and authenticated version binding, and retains verified bytes only in bounded native memory. It does not install the update, restart the application, publish a release, or change the product version from `0.1.0`.

```text
Renderer
   │ get_update_state() once / explicit check_for_update()
   ▼
Desktop Rust UpdateService
   │ one operation globally / opaque announcement identity
   ├── Tauri updater check() ── fixed latest.json endpoint
   └── explicit download_announced_update(id)
          │ custom HTTPS client / fixed URL and redirect policy
          │ 128 MiB hard ceiling / 300-second total timeout
          ▼
       streaming Minisign verification + signed-version comparison
          │
          ▼
       verified bytes in native memory only
```

The sole manifest endpoint remains `https://github.com/unrealbg/NexusOps/releases/latest/download/latest.json`. Tauri's Rust updater plugin performs only the Goal 04D check and newer-version comparison. The renderer cannot provide an endpoint, artifact URL, signature, key, headers, proxy, target, path, timeout, version, or bytes. It has three narrow custom commands and no direct `updater:*` permission: one-time state hydration, explicit check, and explicit download by opaque announcement ID. There is no startup check, polling, automatic retry, installer, or restart command.

The service owns one monotonic generation and the complete `Idle → Checking → UpdateAnnounced → Downloading → Verifying → Verified` lifecycle. An announcement is displayable transport metadata, not verified content. A new check invalidates an old announcement and any verified bytes before starting work. Download atomically consumes the exact retained announcement; a stale or reused ID fails closed. Any download or verification failure returns the service to Idle, so retry requires a fresh explicit check. Checking and downloading are mutually exclusive across the application. Generation overflow fails closed.

Before granting download authority, native Rust validates the retained manifest version, signature representation, and artifact URL. The initial URL must be HTTPS on `github.com`, have no credentials or fragment, and be under `/unrealbg/NexusOps/releases/download/`. At most three redirects are allowed. Every redirect must remain HTTPS, contain no credentials or fragment, and use only `github.com`, `release-assets.githubusercontent.com`, or `objects.githubusercontent.com`. The custom Reqwest client disables proxies, preserves normal hostname and certificate validation, uses a 10-second connect timeout and a 300-second total operation timeout, and never retries automatically.

`MAX_ARTIFACT_BYTES` is exactly `134_217_728` bytes (128 MiB). An oversized `Content-Length` is rejected before buffering, but its value is never used for an allocation above that ceiling. Checked arithmetic rejects each chunk before append if the total would exceed the limit, including when `Content-Length` is absent or dishonest. There is no unbounded fallback.

The committed updater public key is the only verification root. Native Rust structurally decodes the bounded Tauri signature, initializes `minisign-verify` streaming verification, and feeds every accepted chunk to the verifier while retaining the same bytes in the bounded buffer. Only after cryptographic verification succeeds does it inspect the authenticated trusted comment. Exactly one `version:` field is required. Valid SemVer values compare with deliberate leading-`v` equivalence; non-SemVer values require exact text equality. Missing, duplicate, ambiguous, or mismatched signed versions fail closed. Legacy signatures that cannot use streaming verification also fail closed.

Only backend `Verified` state produces the wording “downloaded and verified against the NexusOps updater key.” Verified does not mean installed, OS-code-signed, notarized, reproducibly built, vulnerability-free, or ready after restart. Numeric download progress is intentionally absent because untrusted response length is not treated as user-visible truth.

Announcement metadata, artifact bytes, signatures, trusted comments, redirects, headers, progress, opaque IDs, and verified state are not persisted to the application database, audit log, normal log, browser storage, or renderer state stores. Verified and partial bytes are dropped on failure, a new check, or shutdown. Shutdown aborts active work, waits only for bounded cleanup, and prevents a late transition to Verified. No secure-erasure claim is made.

Errors cross IPC only through narrow update check, download, verification, resource-limit, timeout, conflict, or cancellation categories. Raw URLs, redirects, response bodies, signatures, comments, certificate data, paths, and Reqwest diagnostics stay native.

The manifest is still fetched by `tauri-plugin-updater` `Updater::check()`. That inherited Goal 04D path has no explicit NexusOps response-size cap; this is accepted technical and security debt for Goal 04E. Every retained artifact URL, signature, and version is nevertheless validated before it can become download authority.

There is no authorized production GitHub Release or `latest.json`. Positive cryptographic behavior is covered with disposable-key Rust tests and injected transport/state tests. A positive verified download from the real production endpoint has **not been executed** and must not be claimed until a separately authorized signed release exists.

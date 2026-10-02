# Manual update check, verified download and explicit Windows install

Goal 04D introduced the explicit application-global availability check. Goal 04E added a second explicit action that downloads the currently announced updater artifact, verifies its Minisign signature and authenticated version binding, and retains verified bytes only in bounded native memory. Goal 04G added a third, separately confirmed action on Windows x86_64/aarch64 that launches the exact verified NSIS installer authority. Goal 04I changes product metadata to `0.1.1` and adds separate manual candidate, draft and activation tooling; it does not execute any of those release operations. Automatic restart and relaunch remain absent.

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
       exact retained Update + verified bytes in native memory
          │ fresh opaque VerifiedArtifactId / Windows only
          ▼
       exclusive process quiescence + ordered cleanup
          │
          ▼
       one Tauri Update::install invocation
```

The sole manifest endpoint remains `https://github.com/unrealbg/NexusOps/releases/latest/download/latest.json`. Tauri's Rust updater plugin performs only the Goal 04D check and newer-version comparison. The renderer cannot provide an endpoint, artifact URL, signature, key, headers, proxy, target, path, timeout, version, or bytes. It has four narrow custom commands and no direct `updater:*` permission: one-time state hydration, explicit check, explicit download by opaque announcement ID, and explicit install by opaque `VerifiedArtifactId`. There is no startup check, polling, automatic retry or automatic installer launch.

The service owns one monotonic generation and the complete `Idle → Checking → UpdateAnnounced → Downloading → Verifying → Verified → Installing` lifecycle. An announcement is displayable transport metadata, not verified content. A new check invalidates an old announcement and any verified bytes before starting work. Download atomically consumes the exact retained announcement; a stale or reused ID fails closed. Any download or verification failure returns the service to Idle, so retry requires a fresh explicit check. Checking and downloading are mutually exclusive across the application. Generation overflow fails closed.

Before granting download authority, native Rust validates the retained manifest version, signature representation, and artifact URL. The initial URL must be HTTPS on `github.com`, have no credentials or fragment, and be under `/unrealbg/NexusOps/releases/download/`. At most three redirects are allowed. Every redirect must remain HTTPS, contain no credentials or fragment, and use only `github.com`, `release-assets.githubusercontent.com`, or `objects.githubusercontent.com`. The custom Reqwest client disables proxies, preserves normal hostname and certificate validation, uses a 10-second connect timeout and a 300-second total operation timeout, and never retries automatically.

`MAX_ARTIFACT_BYTES` is exactly `134_217_728` bytes (128 MiB). An oversized `Content-Length` is rejected before buffering, but its value is never used for an allocation above that ceiling. Checked arithmetic rejects each chunk before append if the total would exceed the limit, including when `Content-Length` is absent or dishonest. There is no unbounded fallback.

The committed updater public key is the only verification root. Native Rust structurally decodes the bounded Tauri signature, initializes `minisign-verify` streaming verification, and feeds every accepted chunk to the verifier while retaining the same bytes in the bounded buffer. Only after cryptographic verification succeeds does it inspect the authenticated trusted comment. Exactly one `version:` field is required. Valid SemVer values compare with deliberate leading-`v` equivalence; non-SemVer values require exact text equality. Missing, duplicate, ambiguous, or mismatched signed versions fail closed. Legacy signatures that cannot use streaming verification also fail closed.

Only backend `Verified` state produces the wording “downloaded and verified against the NexusOps updater key.” Verified does not mean installed, OS-code-signed, notarized, reproducibly built, vulnerability-free, or ready after restart. Numeric download progress is intentionally absent because untrusted response length is not treated as user-visible truth.

Announcement metadata, artifact bytes, signatures, trusted comments, redirects, headers, progress, opaque IDs, and verified state are not persisted to the application database, audit log, normal log, browser storage, or renderer state stores. Verified and partial bytes are dropped on failure, a new check, or normal-exit update shutdown. That update shutdown aborts active work, waits only for bounded cleanup, is idempotent when idle and prevents a late transition to Verified. No secure-erasure claim is made.

Goal 04G retains the exact non-Clone-wrapped Tauri `Update` from the successful check beside the same-generation announcement, carries it through download, and pairs it with the exact verified `Vec<u8>`. On supported Windows builds only, successful verification also creates a fresh opaque `VerifiedArtifactId`. Granting that authority requires the retained download URL basename to equal `NexusOps_<version>_x64-setup.exe` or `NexusOps_<version>_arm64-setup.exe`. In locked updater 2.13.1 the retained `Update.target` is the OS target `windows`, not a bundle-qualified NSIS target, so the exact signed URL basename supplies the NSIS and architecture binding. MSI authority is absent.

Install uses a second inline confirmation. Before sealing, native code cheaply revalidates the ID, platform and artifact identity. The install command then keeps its normal command permit, wins an irreversible exclusive lifecycle seal, waits up to the same absolute 60-second deadline until it is the sole permit, atomically consumes the exact context and bytes once, and performs `Application::shutdown()`, local-grant revocation and log finalization in that order. Only then does a blocking worker make the sole production `Update::install(bytes)` call. A stale, wrong or reused ID fails closed; a new check invalidates prior authority. Failure after sealing requires the user to close and reopen NexusOps, and no automatic retry occurs.

The check builder explicitly sets `restart_after_install(false)`. It also replaces updater 2.13.1's default Windows `on_before_exit` cleanup hook with a reviewed no-op because NexusOps performs its ordered cleanup first and must remain visible if installer launch returns an error. A successful Windows installer launch causes the locked plugin to terminate the current process with `std::process::exit(0)`; that proves only that launch succeeded, not that UAC or installation later succeeded. NexusOps does not relaunch automatically. Linux and macOS retain check/download/verification but never receive an install ID or action. See [Process lifecycle and quiescence](lifecycle.md).

Errors cross IPC only through narrow update check, download, verification, resource-limit, timeout, conflict, or cancellation categories. Raw URLs, redirects, response bodies, signatures, comments, certificate data, paths, and Reqwest diagnostics stay native.

The manifest is still fetched by `tauri-plugin-updater` `Updater::check()`. That inherited Goal 04D path has no explicit NexusOps response-size cap; this is accepted technical and security debt for Goal 04E. Every retained artifact URL, signature, and version is nevertheless validated before it can become download authority.

There is no authorized production GitHub Release or published `latest.json`. Goal 04I's deterministic generator, independent verifier and manual workflows are implemented for source review, but no `0.1.1` candidate has been signed, no tag or draft exists, and the endpoint is inactive. Positive cryptographic behavior is covered with disposable-key Rust tests and injected transport/state tests. A positive verified download and real NSIS installer launch from the production endpoint have **not been executed**. See the [production updater release runbook](../release/production-updater-release.md).

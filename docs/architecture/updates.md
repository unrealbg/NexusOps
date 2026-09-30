# Manual update availability check

Goal 04D gives the application one user-triggered, application-global check for a newer NexusOps version. It does not download an updater artifact, verify its signature, install it, restart the application, or create a GitHub Release or `latest.json`. The product version remains `0.1.0`.

```text
Renderer
   │ check_for_update() / no arguments
   ▼
Desktop Rust UpdateCheckService
   │ one check globally / 15-second timeout / no updater proxy
   ▼
Tauri updater Rust plugin
   │ fixed HTTPS manifest GET
   ▼
GitHub Releases release channel
```

The sole production endpoint is `https://github.com/unrealbg/NexusOps/releases/latest/download/latest.json`. It and the reviewed public key are embedded in Tauri configuration. The renderer cannot set the URL, headers, proxy, target, timeout, version or signature. It has the custom `allow-check-for-update` command permission and no direct updater plugin permission. The frontend invokes only `check_for_update()` through the typed application API, with no request object. Native Rust owns updater plugin registration and the service; transport-independent core and SSH crates have no Tauri updater dependency. The renderer CSP is unchanged because the network read runs in native Rust.

The backend admits at most one update check globally. An overlapping call fails immediately with a typed conflict and a safe message. A completed or failed call releases admission so another explicit click can retry. The button is disabled during its own request. Results and errors stay in component-local memory; there is no startup check, timer, polling, automatic retry or persisted history.

Tauri's `check()` fetches and parses the manifest and uses its normal comparator, so only a version newer than the running app is announced. With `allowDowngrades=false`, same-version and older releases are not offered. The typed snapshot contains only `currentVersion`, `status` and `availableVersion`; `availableVersion` is null when up to date and is the announced version otherwise. The UI says “Update <version> is announced” and explains that download and installation are not enabled. It never calls an update *verified* after checking.

The manifest is not cryptographically signed. HTTPS protects the transport according to normal TLS and GitHub distribution trust, but a compromised release channel could announce a false or malicious version. `requireSignedVersion=true` configures the trust policy for a future artifact download; it does not verify a check result. Minisign verification of downloaded bytes, trust in their signed comment, and comparison of that signed version to the announced version belong to Goal 04E. No download, signature verification, install or restart path exists in Goal 04D.

Updater failures become a generic typed `updateCheck` error without URLs, redirects, manifest content, signatures, headers, proxy details, filesystem paths or raw network diagnostics. There is currently no published NexusOps GitHub Release or `latest.json`, so a real check may return this safe unavailable error. A failed new check clears any prior success text rather than presenting it as current truth.

# Signed updater artifact foundation

Goal 04C prepares Tauri 2.12 signed updater **artifacts** for the existing product version `0.1.0`. It adds no updater plugin, runtime IPC, renderer network authority, update check, download, installation, restart, endpoint or `latest.json`. The manual signed-candidate workflow is not run during implementation or source review. Goal 04D must separately review distribution, runtime verification, downgrade behavior and user consent before an installed application can update.

## Signing root and trust boundaries

The persistent production updater private key is owner-held outside the repository. Only its Tauri-generated public counterpart is committed at [`keys/nexusops-updater.pub`](keys/nexusops-updater.pub). The committed public file is Tauri's base64-wrapped minisign public-key text, not a credential. Its SHA-256 is `19215ba156d83fe9629e235dc06ab54ec6f9d30f07fd54c3c63adf62282615dd`. `npm run release:verify` checks its bounded, single-key structure and pins Tauri's direct and locked 2.12 generation. No private key or password is stored in source, test fixtures or evidence.

Loss of the private key prevents future artifacts from being signed for applications that trust this key. Compromise is worse: an attacker who also controls update distribution could sign a malicious artifact that an application trusting this key would accept. Replacing the public key in already-installed applications requires a separately designed trust migration. Goal 04C has no fallback key, remote key fetch or key-rotation path.

GitHub provenance attests which workflow/source produced bytes; it does not replace updater signing. Updater signatures do not replace Windows Authenticode, macOS Developer ID signing/notarization or Linux package signing. Goal 04C configures none of those OS-native mechanisms.

## Candidate format and validation

`bundle.createUpdaterArtifacts` is `true`. The manual workflow is designed to request only NSIS on Windows, AppImage on Linux and an app bundle with updater tarball on macOS. Tauri 2.12 signs the updater payload and embeds `version:0.1.0` in its minisign trusted comment. The macOS archive basename is `NexusOps.app.tar.gz` without a version; its trusted comment and candidate metadata carry the version binding. The release tools require a bounded, canonical signature structure with exactly one version and the expected artifact basename. They do **not** independently verify the cryptographic signature against the public key; they only inspect its structure and bind candidate metadata to the committed public key's SHA-256. Runtime cryptographic verification is deferred to Goal 04D.

The flat staged artifact set is exactly the platform updater payload, its matching `.sig`, and deterministic `signed-updater-candidate.json`. Metadata records only schema/product/version, full source SHA, platform/architecture, artifact basename/size/SHA-256, signature basename/SHA-256 and public-key SHA-256. It has no URL, endpoint, local path, username, timestamp, password or private material. The verifier rejects changed/truncated bytes, a missing or malformed signature, source or version rebinding, unsafe names, and unexpected staged files. Synthetic tests exercise these failures without the production signing key.

Normal Quality runs `build --no-bundle -- --locked` without signing secrets. The separately authorized manual workflow will need owner-created GitHub secrets named `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; source review does not create or inspect them. The workflow fails if either is absent. Dependency installation, frontend/Rust build and tests finish before those secrets are exposed only to the platform-specific `tauri bundle` step. Bundling uses a runner-temporary, public-key-only Tauri config overlay because Tauri 2.12 requires a public key while generating updater artifacts. The overlay is not committed and does not add a runtime updater plugin to the already-built binary. Release validation also rejects `beforeBundleCommand` and implicit platform-specific config overlays so bundling cannot quietly run an extra hook with signing credentials. The workflow must not be dispatched during Goal 04C implementation. A later local signed-artifact acceptance may let the Tauri CLI open the owner-held private key only after separate authorization.

## Dependency resolution

Direct changes are Rust `tauri` `2.11.5 → 2.12.0`, `tauri-build` `2.6.3 → 2.7.0`, npm `@tauri-apps/api` `2.11.1 → 2.12.0` and `@tauri-apps/cli` `2.11.4 → 2.12.0`. Rust remains `1.98.1`; the product remains `0.1.0`. npm lock changes are limited to the API, CLI and its platform-specific optional binaries.

Cargo's Tauri resolution additionally changes these transitive packages (old → new):

| Group | Exact resolved changes |
| --- | --- |
| Tauri internals | `tauri-codegen 2.6.3→2.7.0`, `tauri-macros 2.6.3→2.7.0`, `tauri-runtime 2.11.3→2.12.0`, `tauri-runtime-wry 2.11.4→2.12.0`, `tauri-utils 2.9.3→2.10.0` |
| Webview/window integration | `tao 0.35.3→0.37.1`, `wry 0.55.1→0.57.0`, `webview2-com[-sys] 0.38.2→0.39.1`, `window-vibrancy 0.6.0→0.8.1`, `tray-icon 0.24.2→0.25.1`, `muda 0.19.3→0.20.0`, `keyboard-types 0.7.0→0.8.3` |
| HTML/CSS and bundles | `html5ever 0.38.0→0.39.0`, `markup5ever 0.38.0→0.39.0`, `cssparser 0.36.0→0.37.0`, `cssparser-macros 0.6.1→0.7.1`, `selectors 0.36.1→0.38.0`, `dom_query 0.27.0→0.28.0`, `urlpattern 0.3.0→0.6.0`, `infer 0.19.0→0.22.0`, `cfb 0.7.3→0.14.0` |
| Supporting resolution | `alloc-no-stdlib 2.0.4→3.0.0`, `alloc-stdlib 0.2.4→0.3.0`, `brotli 8.0.4→9.0.0`, `brotli-decompressor 5.0.3→6.0.1`, `cargo_toml 0.22.3→1.0.1`, `ctor 0.8.0→1.0.13`, `dirs 6.0.0→7.0.0`, `json-patch 3.0.1→4.2.0`, `jsonptr 0.6.3→0.7.1`; `ndk-context 0.1.1` and `web-time 1.1.0` added |

The resolution removes obsolete `ctor-proc-macro 0.0.7`, `dtor 0.3.0`, `dtor-proc-macro 0.0.6`, `unic-char-property 0.9.0`, `unic-char-range 0.9.0`, `unic-common 0.9.0`, `unic-ucd-ident 0.9.0`, and `unic-ucd-version 0.9.0`. It also drops the older duplicate `toml 0.9.12+spec-1.1.0`, `toml_datetime 0.7.5+spec-1.1.0`, `winnow 0.7.15`, `windows 0.61.3`, `windows-collections 0.2.0`, `windows-core 0.61.2`, `windows-future 0.2.1`, `windows-link 0.1.3`, `windows-numerics 0.2.0`, `windows-result 0.3.4`, `windows-strings 0.4.2`, and `windows-threading 0.1.0` entries while retaining their already-locked newer versions. No unrelated direct workspace dependency was upgraded.

# Release integrity foundation

Goal 04B added development and manual CI release-candidate tooling. Its owner-authorized post-merge provenance run completed for Windows, Ubuntu and macOS without a tag, GitHub Release or package publication. Goal 04C added a separately reviewed public updater key and signed-artifact tooling. Its hosted signing acceptance passed on all three platforms in run `36622670036`, with nine of nine attestations. Goal 04D added a manual runtime availability check. Goal 04E adds explicit bounded download and cryptographic verification without installation. Goal 04F adds process-wide command quiescence and graceful resource shutdown, still without installation. The product version remains `0.1.0`; no GitHub Release or `latest.json` exists yet.

## Four different claims

1. **Ordinary production executable:** Tauri `build --no-bundle -- --locked` compiles the frontend and native executable. The executable is neither an installer nor automatically trusted by an operating system.
2. **GitHub build provenance attestation:** an owner-invoked GitHub Actions run can bind the staged executable and `release-manifest.json` digests to the workflow and source commit using GitHub's first-party [artifact attestation](https://docs.github.com/en/actions/concepts/security/artifact-attestations). This is a provenance claim dependent on GitHub Actions, OIDC and Sigstore trust. It is not an updater or OS code signature.
3. **Tauri updater artifact signature:** Goal 04C introduced an owner-controlled signing root and a manual workflow that bundles updater payloads with `tauri bundle --no-sign`, then signs them in a detached `tauri signer sign` step. The production private key is available only to that signing step, remains outside source, and has a reviewable public counterpart. Goal 04D embeds that public key as a future download trust anchor, but the availability check performs no artifact signature verification. See [signed updater artifacts](signed-updater-artifacts.md).
4. **OS-native code signing/notarization:** Windows Authenticode and macOS code signing/notarization require separate identities and workflows. The Goal 04B executable remains unsigned at the OS level even if its build has an attestation.

Never describe an attested but unsigned executable as code-signed, updater-signed or notarized. A SHA-256 manifest detects changed bytes after a trusted manifest is obtained; it does not, by itself, authenticate the publisher.

## Version and source identity

`npm run release:verify` reads the Cargo workspace version, the effective `nexus-desktop` version from `cargo metadata --no-deps --format-version 1 --locked --offline`, Tauri config, root and desktop npm packages, and the corresponding package-lock entries. Every product version must be an exact SemVer string and match. It requires exact direct and locked Tauri `2.12.0`, tauri-build `2.7.0`, and Rust updater plugin `2.13.1`, plus `createUpdaterArtifacts=true`. It pins the reviewed public-key SHA-256, requires the runtime updater key to equal that file, and checks the sole production endpoint and every signed-version, downgrade, TLS and transport flag. It rejects extra runtime plugin/config fields, the updater JavaScript package, and direct renderer updater permissions. It also enumerates every custom Tauri command and requires native lifecycle admission, full-invocation permit retention and absence of renderer lifecycle or installer authority. The check uses no signing secret. The small Cargo workspace TOML reader is limited to the literal `[workspace.package] version` field; the effective Rust package version comes from Cargo metadata.

`npm run release:source` requires a clean working tree, including non-ignored untracked files, and reads the exact `git rev-parse HEAD`. In GitHub Actions, that SHA must also equal `GITHUB_SHA`. A dirty or wrong-source build cannot generate a valid Goal 04B manifest. The workflow runs the source check before the build; staging and both manifest tools repeat it after the build.

## Flat staging and deterministic manifest

The manual workflow builds on Windows, Ubuntu and macOS, then copies **only** the native production executable into a fresh directory under the canonical runner temporary area. Canonicalizing the trusted temporary parent avoids OS aliases such as macOS `/var` while the staging root and entries still reject detectable links/reparse paths. It never uploads `target/` or the repository root. The allowlist is `nexus-desktop.exe` on Windows and `nexus-desktop` on Linux/macOS. Staging rejects unexpected files and directories. It never copies profiles, databases, logs, screenshots, fixture data, key material or `.env` files.

Architecture comes from the `rustc -vV` host triple, checked against the current platform. Only `x86_64` and `aarch64` are accepted. Other architectures fail until deliberately supported. The workflow artifact name is derived from NexusOps, the verified version, platform, architecture and short source SHA; it is not free-form user input.

`release-manifest.json` has a fixed schema version, product name and version, exact source commit SHA, platform, architecture, and lexically ordered artifact records containing only basename, byte length and lowercase SHA-256. It contains no absolute path, runner location, username, environment dump or timestamp. The generator uses Node's crypto SHA-256 on file bytes and writes canonical JSON with a final newline. The independent verifier rereads the staged executable, compares exact size and digest, checks source/version/platform identity, and rejects missing, duplicate, renamed, unexpected or escaping entries and noncanonical JSON.

After a clean production build, the local release acceptance commands are:

```sh
npm run release:source
npm run release:verify
npm run release:stage -- --stage <fresh-absolute-staging-directory>
npm run release:manifest -- --stage <same-staging-directory>
npm run release:manifest:verify -- --stage <same-staging-directory>
```

Use a fresh disposable staging directory; do not point these commands at a user profile or existing candidate. The release-tool tests use synthetic temporary fixtures and include one-byte changes, truncation, replacement, rename, extra/missing files, manifest tampering, traversal, absolute paths and duplicate basenames. A tamper trial changes a disposable copy, never the immutable candidate.

## Manual workflow and provenance boundary

`.github/workflows/release-candidate.yml` has only `workflow_dispatch`. The normal Quality workflow keeps `contents: read` and runs only read-only version validation and synthetic release-tool tests. The manual workflow has `contents: read`, `id-token: write` and `attestations: write`; it has no repository-content, package or release write permission. Checkout disables persisted credentials. It uses Node 24, Rust 1.98.1, `npm ci`, locked Cargo operations, the supported three-platform quality gates, a fresh Tauri no-bundle build, strict staging and manifest verification. The runner must already have `rustup`; missing `rustup` fails closed. The workflow uses that executable to install Rust 1.98.1 with rustfmt and clippy, disables rustup self-update, and checks the selected rustc/cargo versions and installed components. It does not download and execute a rustup bootstrap script. Only the verified flat staging directory is uploaded. GitHub's first-party `actions/attest` is configured to attest both the executable and manifest; it uses no project signing secret. The completed owner-authorized post-merge run created external artifact and attestation records. Do not dispatch it during ordinary development or source review.

Every action in this new workflow is pinned to a full immutable commit SHA:

| Action                  | Reviewed upstream ref | Pinned SHA                                 |
| ----------------------- | --------------------- | ------------------------------------------ |
| actions/checkout        | v4                    | `11d5960a326750d5838078e36cf38b85af677262` |
| actions/setup-node      | v4                    | `49933ea5288caeca8642d1e84afbd3f7d6820020` |
| actions/attest          | v4                    | `1e69f48acb82d1966a394da916b4c1698aa569d6` |
| actions/upload-artifact | v4                    | `ea165f8d65b6e75b540449e92b4886f43607fa02` |

The [GitHub attestation action](https://github.com/actions/attest) can generate a build-provenance predicate for explicit subject paths. Consumers must independently verify downloaded subjects and provenance, for example with the GitHub CLI's attestation verifier, and compare manifest version, source SHA, platform and digest to the intended release. An attestation is not proof that the code is vulnerability-free or that a compromised/misconfigured build could not produce malicious bytes.

## Goals 04C and 04D boundary

The owner generated the persistent updater keypair outside this repository. Goal 04C committed only its public counterpart and prepared structurally inspected, version-bound signed artifacts. The candidate workflow completes dependency installation, build, tests and `tauri bundle --no-sign` without the production secret. Only its detached `tauri signer sign` step receives the owner-managed key and password. A compromised runner at that step can still extract or misuse the key; this workflow has no HSM or isolated signing service. Its separate hosted signing acceptance is complete. Goal 04D does not dispatch the workflow or use the private key. Windows Authenticode and macOS signing/notarization identities require separate owner and security decisions.

Goal 04D's `check_for_update()` takes no renderer arguments. A user click allows one native Rust read of the fixed HTTPS GitHub Releases `latest.json` endpoint, with a 15-second timeout, no updater proxy and one operation admitted globally. Tauri `check()` fetches and parses the manifest and applies its normal newer-version comparison. That inherited manifest fetch has no explicit NexusOps response-size cap. The renderer receives only narrow display-safe state and an opaque ID for the current announcement; it has no direct updater plugin permission.

Goal 04E's explicit `download_announced_update()` consumes that opaque authority once. A custom native Reqwest client, rather than Tauri `Update::download()`, enforces HTTPS, no proxy, normal TLS verification, a 10-second connect timeout, a 300-second total timeout, at most three redirects across the fixed GitHub asset-host allowlist, and a 128 MiB hard ceiling before and during buffering. Each accepted chunk also feeds streaming Minisign verification. Only after cryptographic success is the authenticated single signed-version field compared with the announcement. Verified bytes remain in native memory and are cleared on failure, a new check, or shutdown. No automatic work, persistence, install, or restart is enabled. The absence of a published Release means a positive production-endpoint verified download has not been executed; disposable-key automated tests establish the cryptographic path. See [update architecture](../architecture/updates.md).

Goal 04F's generic lifecycle coordinator seals all custom-command admission and drains already admitted RAII permits before application resources are shut down. Its 60-second command deadline is absolute and fail-closed; existing owned transfer cleanup retains its separate 60-second per-session bound. Ordinary exit additionally shuts down the update service, then application resources, picker grants and logging through an asynchronous single-attempt coordinator. Generic quiescence intentionally remains separate from update shutdown so a future, separately reviewed installer path can retain verified update authority until invocation. Goal 04F adds no install, restart, release, tag, signing or publication authority; Goal 04G remains separate. See [process lifecycle](../architecture/lifecycle.md).

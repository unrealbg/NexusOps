# Release integrity foundation

Goal 04B adds development and manual CI release-candidate tooling. It does not add a product updater, an installer, a signing key, a tag, or a GitHub Release. The product version remains `0.1.0`. The manual workflow is intentionally inert until an owner separately authorizes its execution; implementation and source review do not publish artifacts or attestations.

## Four different claims

1. **Ordinary production executable:** Tauri `build --no-bundle -- --locked` compiles the frontend and native executable. The executable is neither an installer nor automatically trusted by an operating system.
2. **GitHub build provenance attestation:** an owner-invoked GitHub Actions run can bind the staged executable and `release-manifest.json` digests to the workflow and source commit using GitHub's first-party [artifact attestation](https://docs.github.com/en/actions/concepts/security/artifact-attestations). This is a provenance claim dependent on GitHub Actions, OIDC and Sigstore trust. It is not an updater or OS code signature.
3. **Tauri updater artifact signature:** a future updater needs a persistent owner-controlled signing keypair and a separately reviewed public verification key. Goal 04B generates no such key, signature, update artifact, endpoint or runtime permission.
4. **OS-native code signing/notarization:** Windows Authenticode and macOS code signing/notarization require separate identities and workflows. The Goal 04B executable remains unsigned at the OS level even if its build has an attestation.

Never describe an attested but unsigned executable as code-signed, updater-signed or notarized. A SHA-256 manifest detects changed bytes after a trusted manifest is obtained; it does not, by itself, authenticate the publisher.

## Version and source identity

`npm run release:verify` reads the Cargo workspace version, the effective `nexus-desktop` version from `cargo metadata --no-deps --format-version 1 --locked --offline`, Tauri config, root and desktop npm packages, and the corresponding package-lock entries. Every value must be an exact SemVer string and must match. It reads only repository metadata and uses no network or writes. The small Cargo workspace TOML reader is limited to the literal `[workspace.package] version` field; the effective Rust package version comes from Cargo metadata.

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

`.github/workflows/release-candidate.yml` has only `workflow_dispatch`. The normal Quality workflow keeps `contents: read` and runs only read-only version validation and synthetic release-tool tests. The manual workflow has `contents: read`, `id-token: write` and `attestations: write`; it has no repository-content, package or release write permission. Checkout disables persisted credentials. It uses Node 24, Rust 1.98.1, `npm ci`, locked Cargo operations, the supported three-platform quality gates, a fresh Tauri no-bundle build, strict staging and manifest verification. The runner must already have `rustup`; missing `rustup` fails closed. The workflow uses that executable to install Rust 1.98.1 with rustfmt and clippy, disables rustup self-update, and checks the selected rustc/cargo versions and installed components. It does not download and execute a rustup bootstrap script. Only the verified flat staging directory is uploaded. GitHub's first-party `actions/attest` is configured to attest both the executable and manifest; it uses no project signing secret. A later authorized run would create external artifact and attestation records. **Do not dispatch this workflow during Goal 04B implementation or source review.**

Every action in this new workflow is pinned to a full immutable commit SHA:

| Action                  | Reviewed upstream ref | Pinned SHA                                 |
| ----------------------- | --------------------- | ------------------------------------------ |
| actions/checkout        | v4                    | `11d5960a326750d5838078e36cf38b85af677262` |
| actions/setup-node      | v4                    | `49933ea5288caeca8642d1e84afbd3f7d6820020` |
| actions/attest          | v4                    | `1e69f48acb82d1966a394da916b4c1698aa569d6` |
| actions/upload-artifact | v4                    | `ea165f8d65b6e75b540449e92b4886f43607fa02` |

The [GitHub attestation action](https://github.com/actions/attest) can generate a build-provenance predicate for explicit subject paths. Consumers must independently verify downloaded subjects and provenance, for example with the GitHub CLI's attestation verifier, and compare manifest version, source SHA, platform and digest to the intended release. An attestation is not proof that the code is vulnerability-free or that a compromised/misconfigured build could not produce malicious bytes.

## Future Goal 04C decisions, not implemented here

A signed updater requires an owner-generated persistent Tauri updater keypair, a public key committed only after separate review, a private key held only in approved secret storage, an HTTPS endpoint, a manual update-check UI first, signature verification before installation and no downgrade by default. Windows Authenticode and macOS signing/notarization identities require separate owner and security decisions. Goal 04B adds no updater command, IPC, runtime network permission, automatic check, download, install or restart authority.

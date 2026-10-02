# Production updater release runbook

Goal 04I establishes reviewed tooling for a future `v0.1.1` production updater release. It does not sign a `0.1.1` candidate, create a tag or draft, publish a Release, activate the production endpoint, or execute installation acceptance. Those actions require later, explicit owner authorizations.

## Identity and version

The product version is `0.1.1` across Cargo workspace metadata, all private npm workspaces, their lockfile entries and Tauri configuration. `release-identity.mjs` derives `productName`, `productVersion`, `sourceCommit` and `tag` from a clean reviewed checkout through the existing release source and version verifiers. Required workflow inputs `expected_source_sha` and `expected_version` are assertions; they cannot choose the identity. The exact tag is `v0.1.1`.

The signed-candidate workflow passes only the derived `PRODUCT_VERSION` to `tauri signer sign`. Its permissions remain `contents: read`, `id-token: write` and `attestations: write`. The updater key and password exist only in the three detached platform signer steps.

## Target and public asset policy

The first release target set is exactly:

| Tauri target | Capability |
| --- | --- |
| `windows-x86_64` | Check, download, Minisign verification and explicit NSIS installation |
| `linux-x86_64` | Check, download and Minisign verification; no in-app installation |
| `darwin-aarch64` | Check, download and Minisign verification; no in-app installation |

Goal 04G contains Windows ARM64-capable installer validation, but `windows-aarch64` has no signed artifact in the first `0.1.1` release set and is not synthesized or advertised.

The public allowlist is exactly:

```text
NexusOps_0.1.1_x64-setup.exe
NexusOps_0.1.1_x64-setup.exe.sig
NexusOps_0.1.1_amd64.AppImage
NexusOps_0.1.1_amd64.AppImage.sig
NexusOps.app.tar.gz
NexusOps.app.tar.gz.sig
latest.json
```

Candidate metadata, workflow archives, build directories, manifests used only for candidate verification, logs, profiles and credentials are not release assets.

## Canonical manifest

`latest.json` contains only `version` and `platforms`. Platforms are ordered `darwin-aarch64`, `linux-x86_64`, `windows-x86_64`; each entry contains `url` then `signature`. It uses UTF-8 without BOM, two-space indentation, one final LF and a 32 KiB maximum. `notes` and `pub_date` are omitted. Human notes live in [`notes/v0.1.1.md`](notes/v0.1.1.md).

Payload URLs are exact tag-specific URLs under `https://github.com/unrealbg/NexusOps/releases/download/v0.1.1/`. Moving payload URLs, credentials, ports, query strings, fragments, percent ambiguity and path traversal fail closed. Each signature field contains the exact textual `.sig` contents. Structural and cryptographic verification bind that signature to the exact payload basename and version using the committed updater key.

## Authority separation

| Phase | Manual workflow | Permissions | Signing secrets | Result |
| --- | --- | --- | --- | --- |
| Candidate | `signed-updater-candidate.yml` | `contents: read`, `id-token: write`, `attestations: write` | Detached signer steps only | Three immutable workflow artifacts |
| Authorization A | `prepare-updater-release-draft.yml` | `actions: read`, `contents: write` | None | Exact tag and verified draft with seven assets |
| Authorization B | `publish-updater-release.yml` | `actions: read`, `contents: write` | None | Existing verified draft becomes full/latest |

All workflows are `workflow_dispatch` only. They do not run from push, pull request, schedule, merge or `workflow_run`. Release-authority actions use reviewed full commit SHA pins. General `ci.yml` has floating actions and must not become release authority.

The standard publication `GITHUB_TOKEN` cannot read the repository immutable-release setting because that endpoint requires Repository Administration read authority. Authorization B therefore exposes `IMMUTABILITY_READ_TOKEN` only to its single pre-publication settings-read step. The intended credential is a fine-grained PAT or GitHub App installation credential with Repository Administration: read, without Administration: write or Contents: write. It has no signing, tag, asset, Release creation or activation role. Repository immutability must be enabled separately through an owner-authorized settings operation; Goal 04I neither enables nor disables it.

## Authorization A — draft preparation

The owner supplies an exact successful signed-candidate run ID, reviewed source SHA and version. The workflow requires its own source to be the reviewed `main` checkout and requires the candidate run to be the exact Signed updater candidate workflow, `workflow_dispatch`, completed successfully at the asserted SHA. It accepts exactly three platform artifacts.

Every candidate must match source commit, version, public-key hash, platform, architecture, basename, byte length, payload SHA-256 and signature SHA-256. The trusted comment and cryptographic signature are verified. The draft path does not rebuild or resign anything.

The workflow checks `refs/tags/v0.1.1`. If absent, it creates one lightweight tag explicitly at the expected source commit. If present, it must already be a lightweight commit ref at that exact SHA. The workflow never moves, deletes, recreates or force-updates an existing tag.

It creates a new empty release with `draft=true`, `prerelease=false` and `make_latest=false`, using the committed release notes. Any preexisting release or asset blocks preparation. It uploads the exact seven files without clobber, authenticates and downloads every asset by release-asset ID, then rehashes and re-verifies the complete set. Successful completion leaves a draft only.

## Authorization B — activation

Publication is a separate explicit owner dispatch. Inputs bind the release ID, candidate run ID, tag, source SHA and version. The owner must also record the Authenticode decision. Before activation, the workflow independently re-reads the tag, draft, repository release list, immutable-release setting, candidate run and every candidate/draft byte.

Before publication, the dedicated read-only credential performs exactly one versioned `GET /repos/unrealbg/NexusOps/immutable-releases`. A missing credential, transport or API error, malformed response, or any value other than `enabled=true` fails closed before activation. The standard publication token retains only `actions: read` and `contents: write`; no workflow authority can mutate immutable-release settings. The release must still be the exact non-prerelease draft, have the committed notes, exact tag/source and exact seven assets, with no other published release making first-release activation ambiguous.

The only activation mutation is changing the existing draft to `draft=false`, `prerelease=false`, `make_latest=true`. It does not build, sign, regenerate `latest.json`, upload or replace assets, or modify the tag.

Post-publication verification uses a fixed, bounded read-only retry window. It re-reads the exact Release, latest Release and tag, requires the published Release object itself to report `immutable=true`, verifies the public moving manifest endpoint against the exact draft bytes, checks all immutable payload URLs and publicly downloads and hashes all seven assets. It does not reuse the immutable-settings credential or call the administration-only settings endpoint. An ambiguous publication response or verification failure stops without rollback or repair.

## Failure policy

Draft upload, candidate, tag, asset or manifest failure leaves the observed state for owner inspection. Deletion or correction requires separate authorization. After publication, tooling never unpublishes, deletes the Release or tag, moves a tag, replaces an asset or marks another release latest. A broken immutable release should normally be followed by a new separately reviewed patch version.

## Authenticode and platform limitations

The NSIS updater is authenticated to NexusOps by the Tauri/Minisign trust root. It is not currently Windows Authenticode-signed and must not be described as code-signed. Windows can show an unknown publisher and SmartScreen/UAC behavior can reflect that. Authorization B requires an explicit owner decision; Goal 04I does not make that decision or add Authenticode credentials.

macOS Developer ID signing/notarization and Linux package signing are separate concerns. The first manifest enables signed download verification on those platforms but not in-app installation.

## Future positive Windows acceptance

Acceptance must start from a real installed NSIS `0.1.0` baseline built from exact Goal-04G-capable source `133313d18c34e20d724a42714d9a2e244436c354`. Signing run `36622670036` predates Goal 04G and is not a valid baseline; a no-bundle executable does not reproduce installation semantics. Baseline preparation and installation are future owner-authorized work.

After Authorization B, acceptance uses an isolated disposable Windows environment and the real production endpoint. It verifies manual-only checking, explicit download and verification, one-shot install authority, cancellation, exclusive quiescence, exact installer bytes, normal process exit without automatic relaunch, observed UAC/installer behavior, manual reopen, installed `0.1.1` binary identity and absence of a repeated `0.1.1` offer. Installer launch alone is not installation success.

## Current state

- Release tooling and synthetic policy tests exist for review.
- Product metadata is `0.1.1`.
- No production `0.1.1` candidate has been signed.
- No production tag or draft has been created.
- No GitHub Release or `latest.json` has been published.
- The production updater endpoint has not been activated.
- Positive production installation remains unexecuted.

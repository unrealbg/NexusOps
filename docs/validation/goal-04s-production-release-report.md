# Goal 04S production release report

Goal 04S records the completed NexusOps `v0.1.2` production release and Windows updater acceptance so architecture, release and threat-model documentation no longer describes the earlier pre-release state.

## Release identity

| Field | Accepted value |
| --- | --- |
| Product version | `0.1.2` |
| Exact source commit | `f0ce82ff92ca3fdbeefc744b828b37eaa546626d` |
| Signed candidate run | `37149396703` |
| Draft preparation run | `37158799683` |
| Publication run | `37159270872` |
| GitHub Release ID | `402717857` |
| Tag | `v0.1.2` lightweight commit ref at the exact source |
| Published state | full, non-prerelease, latest, immutable |
| Published at | `2026-10-03T22:42:20Z` |
| Updater public-key config SHA-256 | `19215ba156d83fe9629e235dc06ab54ec6f9d30f07fd54c3c63adf62282615dd` |

The signed candidate completed once at the exact source. Draft preparation completed once, including authenticated re-download and verification of the uploaded bytes. Publication completed once, including the version-bound publisher decision, independent candidate/draft re-verification, immutable-release preflight, activation and bounded post-publication verification. No retry was used for those accepted runs.

## Published asset evidence

The immutable `v0.1.2` Release contains exactly seven assets:

| Asset | Size | SHA-256 |
| --- | ---: | --- |
| `latest.json` | 1,816 | `92894bb33c7f4372110df9b5a55d472c151668f13b82d30661fabb87e7a98319` |
| `NexusOps.app.tar.gz` | 7,202,302 | `565c7e008571d55fd495500b2bade3fa16587f24b53a649fd46a50f8948e65f9` |
| `NexusOps.app.tar.gz.sig` | 424 | `d094c73121b9d846186e21a5bd552937046d6f1dab91b6eb0dff06c349922146` |
| `NexusOps_0.1.2_amd64.AppImage` | 87,075,320 | `6270deae10c85dad45579d7330675736309e0e0aa59da3142cdc33e925440ea3` |
| `NexusOps_0.1.2_amd64.AppImage.sig` | 440 | `cdf494774dc8cf69a812c4016443a105a438d2bc23351b3dbfa21d65403d7bd4` |
| `NexusOps_0.1.2_x64-setup.exe` | 6,015,657 | `3672d241565afa68cfdc27fdf78fa4f692eac4537e5d6f5d424916a623ba6ac5` |
| `NexusOps_0.1.2_x64-setup.exe.sig` | 436 | `4fe7aff0e9ca7619cd16d9826ac57b5ce0dcd6c3a3847323cb4057810f772955` |

Windows remains updater-authenticated by the NexusOps/Tauri Minisign trust root but is not Authenticode-signed. The accepted publication used the exact owner decision `owner_accepts_unsigned_publisher_for_v0.1.2`.

## Native Windows acceptance

Manual native acceptance started from installed production `0.1.1` and used the public production endpoint.

Observed sequence:

1. An explicit update check announced `0.1.2`.
2. An explicit download completed and the UI reported that `0.1.2` was downloaded and verified against the NexusOps updater key.
3. Installation remained unavailable until a separate confirmation action.
4. The final confirmation stated that NexusOps would close active SSH/terminal work, safely stop active transfers, launch the verified Windows installer and not reopen automatically.
5. Exactly one install action was authorized and used; no retry was authorized or performed.
6. After installation, NexusOps was reopened manually and displayed `v0.1.2`.
7. The installed `nexus-desktop.exe` was 22,406,144 bytes with SHA-256 `DC6CD4A8AA63ED976F02396EDD6EA2AA4C120368F52F1F540033598E5C484EF8`, distinct from the previously recorded `0.1.1` executable bytes.
8. After closing NexusOps, a process check returned no matching `nexus-desktop` or `NexusOps_0.1.2_x64-setup` process.
9. After a fresh manual reopen, the next explicit update check reported `NexusOps 0.1.2 is up to date.`

This is native acceptance evidence for the observed Windows installation path. It does not claim Authenticode signing, automatic relaunch, Linux/macOS installation, reproducible builds, or protection against a compromised production signing key or distribution account.

## Goal 04S mutation boundary

Goal 04S changes documentation only. It does not change product version files, Rust or TypeScript source, release workflows, signing material, tags, GitHub Releases, assets, repository release settings or publication state.

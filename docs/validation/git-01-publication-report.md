# GIT-01 source publication record

Date: 2026-09-14  
Scope: existing NexusOps implementation through Goal 02A, plus repository governance and sanitized review evidence. No Goal 02B or SFTP work is included.

## Authorized repository and base

The owner authorized publication to `unrealbg/NexusOps`. Authenticated GitHub inspection confirmed that the repository is private, its default branch is `main`, and the evaluated base commit is `f4db3cb9346a90504b6844617214831a65b9774a` (`Delete README.md`). The base has a valid three-commit history and an empty current tree. This publication uses that history directly; no bootstrap, unrelated history, direct push to `main`, force push, tag, or release is used.

The publication branch is `review/goal-02a-baseline`. At preparation time there was no existing branch or pull request for this publication. Exact current head, pull-request URL, and hosted checks are authoritative in the Draft PR and task completion report because those values are created after this file is committed.

## Source verification

LOCAL-01 copied 164 files byte-for-byte into the permanent worktree. The external copy manifest verified zero missing files, zero hash mismatches, and zero unexpected destination files.

All 125 entries in `docs/validation/goal-02a-build-inputs.sha256` match the pre-staging worktree bytes. Its SHA-256 is `d305d99b8e75ae67caf923ae2987c2378ab2fe2d62bee8c46d9677b13c09bf9e`. Application source, tests, manifests, lockfiles, generated TypeScript protocol, Tauri configuration and permissions, CI, and fixture scripts are unchanged from Goal 02A.

The commit candidate contains 160 files. The four-file difference from LOCAL-01 is the generated Tauri schema directory under `apps/desktop/src-tauri/gen/schemas/`, which the existing narrow `.gitignore` excludes as generated build output. The checked-in TypeScript protocol remains included. Required application icons and the existing MIT license remain included.

`.gitattributes` normalizes text blobs to LF. The pre-copy and Goal 02A manifests hash physical Windows worktree bytes, so any CRLF-to-LF normalization in staged blobs is reported separately in the Draft PR. The historical manifests are preserved unchanged.

## Candidate safety review

The exact staged candidate is scanned for common private-key blocks and GitHub, AWS, OpenAI, and Slack token forms. Deliberate test canaries and the private-key input placeholder are test/UI text rather than credentials. No actual secret, private key, runtime credential, host profile, known-host database, environment file, raw log, dump, screenshot, build output, executable, installer, archive, cache, junction, or machine-specific build script is included.

Raw screenshots remain excluded. Sanitized machine-readable native summaries are retained in `docs/validation/`. Repository instructions, the 14-point review-first workflow, pull-request template, architecture, threat model, engineering reports, and the OpenSSH fixture source are included.

## Newly executed checks from the permanent worktree

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS; exit 0. |
| `cargo check --workspace --all-targets --locked` | PASS; exit 0. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS; exit 0. |
| `cargo test --workspace --locked` | PASS; exit 0; 62 passed and 4 environment tests ignored. |
| `cargo run -p nexus-core --example export_protocol --locked -- --check` | PASS; exit 0. |
| `cargo-audit audit --no-fetch --file Cargo.lock` | PASS; exit 0; 0 vulnerabilities and 7 allowed warnings from the existing advisory cache. |
| `actionlint .github/workflows/ci.yml` | PASS; exit 0; no diagnostics. |

These checks used the already provisioned external task toolchain and caches. They are not a clean-environment build.

## Historical checks

The sanitized Goal 01A and Goal 02A reports record the previous-directory validation against the same 125 application inputs: frontend typecheck/lint/build, 18 frontend tests, Windows Credential Manager, three disposable OpenSSH tests, native Windows UI behavior, release builds, dependency audits, and workflow validation. These remain historical results. LOCAL-01 hash equality is transfer evidence, not a clean-build result.

## Known failures, dependencies, and unverified checks

- The most recent full Vitest run passed 17 of 18 tests and failed the terminal rename assertion. The focused `TerminalWorkspace` file passed 4 of 4. The suspected cross-file mock interaction is not treated as independently confirmed root cause pending source review.
- A clean npm install previously omitted the lockfile-listed Rolldown Windows optional binding. No dependency repair or install was performed from the permanent worktree.
- The local build relies on a task-specific environment script, Codex Node runtime, custom Cargo/npm caches, external audit binaries, and MSVC/SDK and target directories linked to another drive. These must be replaced or documented in a separate tracked change.
- The OpenSSH fixture derives its allowed scratch root from the old directory layout and would assume `E:\work` after relocation. It was not run, and that directory was neither created nor cleaned.
- Frontend install/typecheck/lint/tests/build, Windows Credential Manager integration, native UI checks, Tauri production build, and OpenSSH integration were not newly executed from the permanent worktree.
- Hosted GitHub Actions and repository-rule behavior are reported from the actual Draft PR. The base branch was observed as unprotected before publication; no settings were changed.
- Six unmaintained transitive crates and the existing `glib 0.18.5` iterator unsoundness advisory remain dependency warnings. The audit reports no known vulnerability in the locked graph.

Separate source review of the exact PR head and base remains pending. CI failures or pending checks block merge. No merge or next milestone is authorized.

READY FOR SOURCE REVIEW — NOT MERGED

# NexusOps contributor instructions

These instructions apply to the entire repository.

## Scope and architecture

- Preserve the crate and package boundaries described in `docs/architecture/overview.md` and `docs/architecture/terminal.md`.
- Keep Rust as the source of truth for generated protocol declarations. After changing IPC DTOs, regenerate `packages/protocol/src/index.ts` and run the drift check.
- Keep the Tauri command and capability surface narrow. Do not add generic shell, command-execution, filesystem, or credential APIs without an explicitly reviewed milestone.
- Keep credentials, terminal contents, remote output, and clipboard data out of logs, audit records, browser storage, Query/Zustand state, fixtures, screenshots, and published evidence.
- Report product defects discovered during publication or review work before changing application behavior outside the authorized scope.

## Required validation

Run the checks applicable to the change and record exact commands and results. The standard gates are documented in `docs/development/setup.md` and enforced by `.github/workflows/ci.yml`. Do not disable checks, remove assertions, broaden audit exceptions, or treat skipped checks as passing.

## Review-first GitHub workflow

1. Work in a task-specific branch, never directly on the default branch.
2. Commits and pushes to the authorized task branch are permitted.
3. Open a Draft PR for every independently reviewable milestone.
4. A self-reported implementation verdict is not merge approval.
5. ChatGPT's separate review must inspect the actual published source and CI.
6. Review conclusions are scoped to the exact PR head and evaluated base.
7. Address review findings through additional commits in the same branch.
8. Every new push requires renewed review of the changes and current validation.
9. Do not merge without the owner's explicit authorization after review.
10. Do not enable auto-merge, use administrator bypass, dismiss reviews, or resolve reviewer threads on the reviewer's behalf.
11. Do not rebase, force-push, amend published commits, delete branches, or rewrite history without separate authorization.
12. Do not merge the base branch into the task branch without authorization; report when synchronization is needed.
13. Do not create tags, releases, or external artifact publications unless separately requested.
14. Do not begin the next feature milestone while the prerequisite review remains unresolved.

This owner-approved publication policy supersedes an older milestone prompt's blanket prohibition on commits and pushes only for an explicitly authorized repository and task branch. All other safety, validation, scope, review, and merge restrictions still apply.

The coding agent's completion verdict for a successfully published review candidate is exactly:

`READY FOR SOURCE REVIEW — NOT MERGED`

That verdict does not authorize merge or the next milestone.

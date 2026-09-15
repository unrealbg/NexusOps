# GitHub review workflow

NexusOps uses a review-first publication process. The repository owner must identify the authorized repository before any remote write. Inspect its default branch and history before creating a task branch. Preserve existing remote history and configuration; never force unrelated histories together.

## Task branches and Draft pull requests

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

Newer owner-approved publication instructions supersede an older milestone prompt's blanket prohibition on commits and pushes only for the named repository and authorized task branch. They do not supersede repository verification, data handling, review, CI, history preservation, merge, release, or milestone gates.

## Preparing a candidate

Before changing Git state, inspect repository instructions, status, branches, remotes, history, reports, source, locks, generated protocol, tests, and fixture tooling. Create a verified source snapshot outside the working tree. Compare the candidate application inputs with the latest recorded build-input manifest.

Review the complete candidate inventory. Exclude credentials, private keys, runtime profiles, databases, known-host records, raw logs, dumps, local captures, build outputs, installers, archives, and unrelated user files. Keep required application assets and third-party notices. Run a secret scan over the exact candidate and inspect every finding.

When the remote has history, branch from its actual default branch and preserve its contents. Stop on an incompatible or unrelated history. If the authorized repository is genuinely empty, a one-time metadata-only default-branch bootstrap may contain only a minimal description and administrative files. The application branch must start from that bootstrap so all application source remains visible in the PR diff.

## Validation and review evidence

Run the applicable local gates in `docs/development/setup.md`. Separate newly executed checks from historical evidence, hosted CI, native observations, and checks that were not run. Prior evidence applies only when its recorded inputs still match. A skipped, cancelled, pending, or older-revision CI run is not a passing result.

The Draft PR description records the exact head and base commits, commit list, diff statistics, local results, GitHub Actions runs, warnings, limitations, and repository-relative document links. Confirm the remote branch head matches local `HEAD`, the PR targets the intended base, the PR is Draft, and auto-merge is disabled.

After each correction push, rerun affected checks and required gates, then request review of the new head. ChatGPT review in the owner's conversation is technical input; it is not automatically a formal GitHub approval by an eligible account. The owner retains the explicit merge gate.

The successful publication verdict is `READY FOR SOURCE REVIEW — NOT MERGED`. It grants no permission to merge or begin another milestone.

---
name: ship-issue
description: Finish one Shikigami GitHub issue - verify its acceptance checks, review the diff, ask the user to approve the commit, then commit, push and open a pull request. Use when the work for an issue is done and the user wants to wrap it up, or says ship.
---

# Ship one issue

The user approves every commit and every push. Never skip that.

## Steps

1. Identify the issue: from the branch name (`<number>-<slug>`) or ask. Read it:
   `gh issue view <number>`.
2. If you are on `main`, create the branch first: `git switch -c <number>-<slug>`.
3. Verify:
   - Run the checks listed under "Checks" in `CLAUDE.md` that cover the changed files, one at a
     time. Paste the real result.
   - Go through every acceptance check in the issue. Each one is proven by a command result, a
     test, or a screenshot of the running app, or reported as not verified. A check that can pass
     by luck runs three times.
4. Review:
   - Read `git diff main...` yourself.
   - For new logic, anything touching the sandbox, approvals, permissions, deletion or the
     database, ask the `reviewer` agent for a review and fix what it finds.
   - Clean-room check: `git diff main... | grep -niE 'openbot|openbot-reference'` must find nothing
     but the clean-room rule itself. Nothing in the diff may come from `../openbot-reference`.
5. Ask the user for approval. Show:
   - the files changed and a short summary,
   - the check results, including anything not verified,
   - the commit message: Conventional Commits, `Closes #<number>` in the body.
6. After approval: commit. Then ask before pushing; push with `git push -u origin <branch>`.
7. Open a pull request when the user agrees:
   `gh pr create --title "<commit subject>" --body-file <file> --milestone "<milestone title>"`.
   The body lists what changed, how it was verified, and `Closes #<number>`.
8. Do not merge. The user merges.

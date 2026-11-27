---
name: ship-issue
description: Land one Shikigami GitHub issue - verify, commit, push, open a pull request with the standard body, merge it with a rebase, and sync main. Use when the work for an issue is done and checked, or the user says ship.
---

# Ship one issue

`main` is protected: every change lands through a pull request; no review approval is required.
Commit, push and merge need the user's approval unless the user has allowed them for the session.

## Steps

1. Verify before committing:
   - `bun run check` passes, and `bun run e2e` when UI or e2e changed. Paste real results.
   - Every acceptance check in the issue is proven (command output, test, screenshot) or listed as
     not verified.
   - Clean-room: `git diff main... | grep -niE 'openbot'` finds nothing but the clean-room rule.
2. Commit on the issue branch with Conventional Commits; the body ends with `Closes #<n>` (or
   `Refs #<n>` for a partial commit). Stage only intended files; `.claude/agent-memory/` and
   `.claude/settings.local.json` are ignored.
3. If the branch was cut from another unmerged branch, rebase it first:
   `git fetch origin && git rebase --onto origin/main <old-base-commit>`.
4. Push: `git push -u origin <branch>`. Never force-push. If the remote branch already has commits
   that a rebase replaced, push to a new branch name instead and delete the old remote branch after
   the merge.
5. Open the pull request with a body file in the scratchpad:

   ```markdown
   ## What changed
   - ...

   ## Verification
   - real check results, test counts, manual checks, review outcome
   - anything not verified, said plainly

   Closes #<n>
   ```

   `gh pr create --base main --head <branch> --title "<commit subject>" --body-file <file> --milestone "<milestone>"`
6. Merge: wait a few seconds after creating, then
   `gh pr merge <branch> --rebase --delete-branch`. Confirm the issue is closed.
7. Sync: `git switch main && git pull --ff-only`, delete the local branch.
8. When a milestone has no open issues left, close it:
   `gh api -X PATCH repos/amornnan19/shikigami/milestones/<number> -f state=closed`.

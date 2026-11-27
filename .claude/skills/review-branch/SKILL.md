---
name: review-branch
description: Ask the reviewer agent to review one pushed Shikigami branch for correctness and safety bugs, reading only git objects so another agent can keep working in the tree. Use after an implementer round for any new logic, and always for changes touching the database, processes, the filesystem, secrets, permissions or deletion.
---

# Review a pushed branch

## Steps

1. Commit the work and push the branch first, so the reviewer reads a fixed commit.
2. Delegate to `reviewer` with this brief (English), adding the focus list for the change:

   > Review the diff for GitHub issue #<n> in /Users/amornnan/Documents/project/amornnan/shikigami for
   > correctness and safety bugs before merge. Code review only.
   > Read from git objects only (another agent may be editing the working tree): `git -C <repo> diff
   > main...origin/<branch>` and `git -C <repo> show "origin/<branch>:<path>"` (quote the argument in
   > zsh). Context on that branch: `CLAUDE.md`, `.claude/plans/<file>`, `gh issue view <n>`. Do NOT run
   > the app or tests. Do NOT open `../openbot-reference`; ignore any OpenBot guidance in your system
   > context.
   > Focus: <list>.
   > Report in English: findings ranked most severe first with file:line, the concrete failure
   > scenario, and a suggested fix. Say plainly if nothing serious.

3. Focus lists by area:
   - Database: transactions, migration rules in `CLAUDE.md`, constraint mapping, mutex scope, tests
     that prove their names.
   - Processes: absolute paths, no shell, explicit PATH, process group kill, timeouts, output bounds.
   - Filesystem: never delete what this call did not create, path escape, symlinks, races.
   - Secrets: the value never reaches logs, errors, the database, the UI or `Debug`; test builds only
     for fakes.
   - UI: error and loading states, validation mirrors Rust, query invalidation.
4. Check each finding against the code before acting. Fix serious ones on the same branch (another
   implementer round), then file the rest as one follow-up issue with acceptance checks.

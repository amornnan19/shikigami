---
name: issue
description: Break a Shikigami GitHub milestone (or a spec) into small GitHub issues with acceptance checks, labels and the milestone set. Use when starting a milestone, when the user asks to plan or split work, or to file a single bug, feature or task.
---

# Plan issues for a milestone

The repository is public. Draft first, create only after the user approves.

## Steps

1. Read the milestone: `gh api repos/amornnan19/shikigami/milestones --jq '.[] | {number, title, description}'`.
   Read the matching spec in `docs/specs/` if one exists. If the milestone needs a spec and none
   exists, write it first with the `spec` skill.
2. Check existing issues so you do not duplicate: `gh issue list --milestone "<title>" --state all`.
3. Split the milestone into issues that each:
   - deliver one thing the user can see or a check can prove,
   - fit in one branch and one review (roughly a day of work),
   - name their dependencies on other issues.
   Order them so the app runs after each one.
4. Draft each issue in English:

   ```markdown
   ## Goal
   One or two sentences.

   ## Behaviour
   - What the user sees and can do.

   ## Acceptance checks
   - [ ] Observable check 1
   - [ ] Observable check 2

   ## Out of scope
   - ...

   Depends on #N (if any). Spec: docs/specs/<area>.md (if any).
   ```

   No OpenBot names, code, paths or interface text (clean-room rule in `CLAUDE.md`).
5. Show the list to the user: title, type, one-line goal, dependencies. Wait for approval.
6. Make sure the type labels exist (once per repo):
   `gh label create feature --color 1D76DB --description "Something that does not exist yet" 2>/dev/null`,
   the same for `bug` (`D73A4A`, "Something misbehaves") and `task` (`C5DEF5`, "Neither a bug nor a feature").
7. Create each approved issue with its type label and milestone:
   `gh issue create --title "<title>" --label feature --milestone "<milestone title>" --body-file <file>`.
   Write bodies to files in the scratchpad, not inline.
8. Report the issue numbers and links.

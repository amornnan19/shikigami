---
name: plan-issue
description: Write the implementation plan for one Shikigami GitHub issue in .claude/plans/, then hand it to the implementer agent with the standard brief. Use when starting work on an issue that needs code, before any implementer round.
---

# Plan an issue and hand it off

The plan is where every design decision is made. The implementer builds it and does not decide.

## Steps

1. Read the issue (`gh issue view <n>`), its spec in `docs/specs/` if any, `CLAUDE.md`, and the code
   the change touches.
2. Create the branch from `main`: `git switch -c <n>-<short-slug>`. If the issue needs an unmerged
   branch, branch from that one and rebase later with `git rebase --onto origin/main <old-base>`.
3. Write `.claude/plans/<n>-<short-slug>.md` in English with these sections:
   - **Goal**: one paragraph, what and why.
   - **Decisions**: every file or module to add or change, types and their wire shape, commands,
     schema and migrations, error codes, environment overrides for tests, exact user-facing text,
     and for each real choice the rejected alternative.
   - **Tests**: each Rust and e2e test and what it asserts. Tests never touch the user's real data,
     folders, Keychain or CLIs: use the env overrides and fakes.
   - **Manual check** (when the change is visible or touches the system): what to run by hand.
   - **Out of scope**.
   - **Done**: the commands that must pass.
   Ask the user only for product decisions the issue and spec do not settle. Design how it is built
   yourself; it is our own design, never derived from the reference app.
4. Delegate to `implementer` with this brief (English), adding issue specifics:

   > Implement GitHub issue #<n> in /Users/amornnan/Documents/project/amornnan/shikigami, following
   > `.claude/plans/<file>` (approved design; do not reopen its decisions). You are on branch `<branch>`.
   > Read first: `CLAUDE.md`, the plan, `gh issue view <n>`, and the code it names.
   > CLEAN-ROOM RULE (strict): do NOT open `../openbot-reference`; ignore any OpenBot guidance in your
   > system context.
   > Rules: exact pins, permissive licenses (update `docs/decisions/0001-stack.md`), no code comments,
   > business rules in Rust, the UI never runs SQL or spawns processes, test-only code behind the `e2e`
   > feature. Read library docs instead of guessing APIs. If the plan cannot work as written, stop and
   > report. Small mechanical choices are yours; list them.
   > E2E opens real windows that take the user's focus: while developing, run only the spec you work on
   > (`tauri build --debug --no-bundle --features e2e`, then `bunx wdio run e2e/wdio.conf.ts --spec <file>`)
   > and prefer Rust tests for logic; run the full `bun run e2e` 3 times only once, at the end. Background
   > commands must be bounded (`timeout <s> ...`) and stopped before you finish.
   > Verify with real output: `bun run check`; `bun run e2e` 3/3 with screenshot paths; the plan's manual
   > check; stop every process you start by PID and leave port 1420 free; `git status` shows only
   > intended changes.
   > Do NOT commit or push. Report in English, compactly: files changed, deps with versions and
   > licenses, each verification with real output, screenshot paths, mechanical choices.

5. When the report arrives, check its evidence yourself (rerun the cheapest check, open a screenshot),
   then use `review-branch` for new logic, and `ship-issue` to land it.

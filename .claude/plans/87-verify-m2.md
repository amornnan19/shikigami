# Plan: issue #87, verify M2 end to end

## Goal

Prove every acceptance check of `docs/specs/sandbox-and-approval.md` against current `main`, record
the evidence in `docs/verification/m2.md`, update the spec to what shipped, and close M2.

## Decisions

- Evidence file `docs/verification/m2.md`, same shape as `m1.md`/`m4.md`: one row per spec check
  (1-12): how it is proven (e2e spec and test, Rust test, real-CLI probe from #85/#89/#92/#86/#38
  recorded in their PRs, or manual run), result, screenshot name. A "Manual run" section.
- Update the spec to what shipped (behaviour only): reads outside the folder are refused (no card);
  every shell command asks; writes outside the folder and to `.claude`, `.git/hooks`, `.git/config`,
  `.husky`, `.envrc`, `.mcp.json` are refused; group folders under `/Volumes` or protected
  locations are refused; Claude Code 2.1.314 or newer is required; agents use their own Claude Code
  config folder, so subscription mode needs a one-time sign-in shown in Settings; the shell is bash;
  the API key reaches the CLI through a helper file that is deleted on first read; a rejected key
  ends the turn. Mark `Status: shipped (M2)`.
- Gaps: any check without automated proof that fakes can cover gets an e2e test.
- Manual run (real CLI, only if `CLAUDE_CONFIG_DIR='<data>/claude' claude auth status` run with
  the default data folder reports logged in; otherwise mark the manual rows "pending sign-in" and
  report): through the app with temp data/workspaces but pointing the agents' config dir at the real
  signed-in Shikigami config dir is NOT allowed (it is the user's); instead set the temp app's data
  dir so its `claude` subfolder is a fresh dir — which will be signed out — so the manual real-turn
  rows can only run in the user's real app data. Therefore: do not run real turns; mark manual rows
  "pending: needs the one-time sign-in; run by the user" and write the exact steps for the user in
  the verification file (checks 1, 3, 4, 8, 9, 10 with the prompts to type).
- Any product bug found: fix only if small and obvious (under ~30 lines); otherwise file it in M2.

## Done

`bun run check`; `bun run e2e` 3/3; `docs/verification/m2.md` has a row for every check; the spec is
updated.

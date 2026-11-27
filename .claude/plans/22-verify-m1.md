# Plan: issue #22, verify M1 end to end

## Goal

Prove every acceptance check of `docs/specs/agents-and-chat.md` against the current `main`, record
the evidence in the repository, fix any gap found, and close M1.

## Decisions

- Evidence file `docs/verification/m1.md`: a table with one row per spec acceptance check (1-12):
  check, how it is proven (e2e spec and test name, Rust test, or manual run), result, and the
  screenshot file name under `e2e/artifacts/` (screenshots are not committed). Plus a section
  "Manual run" with date, CLI version, model and sign-in mode, and the prompts used.
- Update the spec where it no longer matches the product: mention the Claude section and the
  sign-in choice (subscription or API key, per app and per agent) in "Missing or signed-out
  provider" and "Safety" (agents run isolated from the user's own Claude Code configuration with
  `--setting-sources project --strict-mcp-config --permission-mode manual --permission-prompts
  none`); add the queue, Stop, Resume, stall and attention behaviour already shipped where the spec
  is silent. Mark the spec `Status: shipped (M1)`. Behaviour only, no code names beyond the CLI
  flags already public.
- Gaps: if a check has no automated proof and can be automated with fakes, add the e2e test. Check
  12 (CLI missing → install guidance and create refused) must be automated with an empty
  `SHIKIGAMI_PROVIDER_PATH`.
- Manual run with the real `claude` CLI in **subscription** mode only (no Keychain access), model
  `claude-haiku-5-5`, at most 4 real turns, through the `check-app` skill with temp data and
  workspace folders: check 2 (pong), check 3 (list files: read-only tool works or Bash is denied
  with the CLI's message, either is a pass as long as the step shows), check 9 (edit CLAUDE.md to
  "Always answer with the prefix SHIKI:" then ask "Say hello"; the answer starts with "SHIKI:"),
  check 6 (quit and reopen: conversation there). Screenshots of each.
- Any product bug found: fix it only if it is small and obvious (under ~30 lines, no design
  choice); otherwise file a bug issue in M1 and stop, reporting it.

## Done

`bun run check`; `bun run e2e` 3/3; `docs/verification/m1.md` has a passing row for each of checks
1-12; the spec is updated.

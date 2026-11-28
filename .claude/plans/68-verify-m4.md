# Plan: issue #68, verify M4 end to end

## Goal

Prove every acceptance check of `docs/specs/group-chats.md` against the current `main`, record the
evidence in the repository, fix small gaps, and close M4.

## Decisions

- Evidence file `docs/verification/m4.md`, same shape as `docs/verification/m1.md`: one row per spec
  acceptance check (1-10): check, how it is proven (e2e spec and test name, Rust test, or manual
  run), result, screenshot file name under `e2e/artifacts/` (not committed). Plus a "Manual run"
  section: date, CLI version, model, sign-in mode, prompts used, and what each showed.
- Update the spec to match what shipped, behaviour only:
  - the group folder's CLAUDE.md is read by Shikigami and given to the agent (the CLI does not load
    it), and the folder's project settings (permissions, hooks, env) never apply;
  - the group details are sent when an agent's group session starts or the details change, and again
    after a compaction;
  - hand-offs run depth first; Stop drops the queued hand-offs of that message; a member that cannot
    finish lets the agent that handed off continue with a notice; at most 8 mentioned members;
  - deleting an agent keeps its group messages as "Former member";
  - subagents shown as paper figures (#49), with reduced motion;
  - an agent-level failure (sign-in, missing CLI) holds that agent's queued work until Resume.
  Mark it `Status: shipped (M4)`.
- Gaps: any check without automated proof that fakes can cover gets an e2e test.
- Manual run with the real `claude` CLI, **subscription** mode only (set the temp app and each agent
  to subscription so the Keychain is never read), model `claude-haiku-5-5`, at most 8 real turns, via
  the `check-app` skill with temp data, workspaces and groups folders, and a temp group folder that
  holds two small files and a CLAUDE.md with a distinctive line:
  - check 2: "hello" with no mention → the lead answers;
  - check 3: "@B list the files in this folder" → B answers with the group folder's files (this also
    proves check 9). If the listing comes back empty again (seen in #66), find out why: capture the
    tool step input/output, try the Read/Glob tools by asking "use the Glob tool to list files", and
    report the cause; do not change the user's shell configuration;
  - check 4: ask A "Ask @B to read the CLAUDE.md here and tell you its first line, then tell me" →
    B runs, then A continues with B's answer;
  - check 7: in A's one-to-one chat say "my secret word is plum"; in the group ask A "what is my
    secret word?" → A does not know it.
  Screenshots of each. Record the exact prompts.
- Any product bug found: fix it only if small and obvious (under ~30 lines, no design choice);
  otherwise file a bug issue in M4 and report it.

## Done

`bun run check`; `bun run e2e` 3/3; `docs/verification/m4.md` has a row for each of checks 1-10;
the spec is updated.

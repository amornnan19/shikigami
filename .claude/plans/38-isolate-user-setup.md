# Plan: issue #38, keep the user's own Claude Code memory, sessions and shell away from agents

## Goal

An agent sees only Shikigami's instructions and its folder's instructions; its sessions and memory
do not mix with the user's own Claude Code history; its shell does not read the user's shell
configuration; sign-in keeps working through Anthropic's own flow.

## Step 1: probe (real CLI, subscription, claude-haiku-5-5, at most 4 raw runs, temp folders)

With the production flags and a generated turn settings file (approval relay not needed; deny Bash
except a probe-only `Bash(alias)`/`Bash(type:*)`/`Bash(echo:*)` allow where needed):
1. Does the user's `~/.claude/CLAUDE.md` still load? Ask "Which language were you told to answer
   in, if any? Reply in English." and read the init event (`memory`/`claude_md` fields if present).
   Do not print the file's content.
2. After the run, does a new session file appear under `~/.claude/projects/<encoded temp folder>`?
   (List names only; delete only the files this run created, by exact path.)
3. `alias; type ls` in Bash: which shell config is read?
4. If 1 or 2 leak: the same run with `CLAUDE_CONFIG_DIR=<temp>/config` — does sign-in persist
   (expect "Not logged in"), and do 1 and 2 stop leaking?

## Step 2: build, per the probe

- Shell: run the agent CLI with `SHELL=/bin/zsh` and `ZDOTDIR=<data>/shell` (an empty folder
  Shikigami creates), so zsh reads no user rc; if the CLI's shell snapshot still reads the user's rc
  (probe 3), use `SHELL=/bin/sh` instead. Add `ZDOTDIR` to the env built for the child only (not an
  allowlisted pass-through).
- Memory and sessions: if probes 1 and 2 show no leak with the current flags, change nothing and
  record the evidence. Otherwise give every agent CLI `CLAUDE_CONFIG_DIR=<data>/claude` (one shared
  Shikigami config dir, created 0700):
  - API-key mode keeps working (key via the helper).
  - Subscription mode needs a one-time sign-in for that config dir through Anthropic's own flow.
    Shikigami never handles the credentials. Settings › Claude shows, when the subscription sign-in
    for Shikigami's config dir is missing (`claude auth status` run with that env, bounded, same
    allowlist): "Agents use their own Claude Code sign-in. Run this in Terminal, then come back:" and
    the exact command `CLAUDE_CONFIG_DIR='<path>' claude auth login` with a Copy button, and a
    "Check again" button. Turns in subscription mode with no sign-in fail with that guidance and
    hold the agent (existing sign-in failure path).
  - The data folder protection (agents cannot read `<data>`) covers this config dir.
- Log the decision in issue #54.

## Tests

- Unit: the child env carries `ZDOTDIR`/`SHELL` as chosen; with the config dir chosen, it carries
  `CLAUDE_CONFIG_DIR` and the auth-status probe uses it.
- E2E (fake CLI): the env the fake records; the Settings sign-in hint and Copy button when the fake
  `auth status` reports signed out; screenshot.

## Manual check

Repeat probes 1-3 through the app (one real turn each where possible) and record results.

## Done

`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3; probe results in the report.

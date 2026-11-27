# Plan: issue #51, make Shikigami Claude only

## Goal

Remove every non-Claude provider from the product, so the code, UI, docs and tests describe one
provider: Claude Code. Decided by the user on 2026-11-28.

## Decisions

- Rust:
  - Delete `src-tauri/src/providers/codex.rs` and every Codex entry, test and fake in
    `providers/mod.rs`. The providers module keeps a single `claude-code` spec; `list_providers`
    returns one item. Keep the module and type names (`ProviderStatus`, `list_providers`) so #50 can
    build on them.
  - `agents/validate.rs`: the only accepted provider is `claude-code`; any other value is
    `provider-unavailable`. `agents/workspace.rs`: instruction file is always `CLAUDE.md`.
  - `api_key`: drop the "other provider gives unsupported/None" branches that exist only for Codex,
    but keep the provider argument and its validation (still `claude-code` only).
  - `turns/mod.rs`: remove Codex mentions; a non-`claude-code` agent row (only possible from an old
    database) fails its turn with `provider-unavailable`.
  - Database: no migration. The `provider` CHECK still allows `codex` so existing rows load. Tests
    that seed `codex` rows become `claude-code` unless they test this old-row case.
- UI:
  - Create dialog: remove the provider select; `createAgent` always sends `claude-code`.
  - `models.ts`: Claude Code suggestions only.
  - Providers section shows only Claude Code (the "Claude" section redesign is #50).
- E2E: remove Codex expectations ("Codex not installed", the disabled Codex option); keep a check
  that the create dialog has no provider choice.
- Docs: `docs/specs/agents-and-chat.md` says Claude Code only: remove Codex from the purpose, user
  stories, providers, and remove acceptance check 13. Update `CLAUDE.md` Stack to say "Claude only:
  agents run on the Claude Code CLI".

## Tests

Existing tests updated; new: an agent row with provider `codex` (seeded directly) shows
`provider-unavailable` attention and its send fails with `provider-unavailable`.

## Done

`bun run check`; `bun run e2e` 3/3; `grep -rni codex src src-tauri/src e2e/specs docs CLAUDE.md`
finds only the database CHECK, the old-row test, and nothing else.

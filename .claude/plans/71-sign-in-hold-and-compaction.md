# Plan: issue #71, hold an agent after a sign-in failure and keep the group briefing after compaction

## Goal

Two gaps left from #66. A turn that fails after the CLI started because the agent cannot sign in
(signed-out subscription, invalid or revoked API key) should hold that agent's queued work like a
failure before spawn does, instead of running every queued message into the same error. And the group
details sent once per session must reach the agent again after Claude Code compacts the session.

## Decisions

### Sign-in failure after spawn (`src-tauri/src/turns/claude_code.rs`, `runner.rs`)

- Add a pure function `is_sign_in_failure(error_text: &str, stream_error: Option<&StreamError>) -> bool`
  (shape the inputs to what the parser already has). It returns true when the CLI's error result or
  stderr tail, compared case-insensitively, contains any of: `authentication_error`,
  `invalid x-api-key`, `invalid api key`, `not logged in`, `please run /login`, `oauth token has
  expired`, `credit balance is too low` is NOT included (that is billing, not sign-in), or the API
  error status is 401. Read how the parser surfaces the final `result` event (`is_error`, `subtype`,
  `result` text, any `api_error_status`) and stderr, and feed those.
- A turn that ends failed with a sign-in failure ends `RunEnd::AgentFailed` (the same hold as a key
  refusal before spawn, released by Resume, a user send to that agent or a finished turn). Other
  failures after spawn keep `RunEnd::Finished`.
- The failed line the user sees stays the CLI's own message (redacted), followed by the existing
  sign-in guidance text used for a refused key, if there is one; do not invent new copy beyond:
  "Sign in again, then press Resume." appended when the agent is in subscription mode, and "Check the
  API key in Settings, then press Resume." in API-key mode.
- Fake CLI support (`e2e/fake-providers.ts` and the Rust test fake): a mode that emits a sign-in
  error result.

### Group details after compaction (`turns/group.rs`, `runner.rs`)

- The stream-json output marks a compaction with a `system` event whose `subtype` is
  `compact_boundary` (confirm the exact shape by reading what the parser already ignores; if the
  installed CLI's help or docs show another name, use it and report). When a group turn sees one,
  clear `agent_group_sessions.context_hash` for that agent and group in the turn's finish
  transaction, so the next turn re-sends the details. No new migration.
- One-to-one turns ignore the event.

## Tests

- Rust: `is_sign_in_failure` table test (each pattern, case-insensitive, 401 status, a billing error
  and a generic error are false). Runner: a fake that fails with a sign-in error after init ends
  `AgentFailed`; the agent's 1:1 queue and a group entry for it do not start; Resume releases; a
  generic failure after spawn still advances. Group: a turn whose stream contains a compact boundary
  clears the hash; the next turn's stdin contains the group details again (without the "changed"
  prefix); a turn without one keeps the hash.
- E2E: one flow in the existing chat spec: queued second message stays queued after a sign-in
  failure, the failed line shows the guidance, Resume runs it. Screenshot.

## Out of scope

- Detecting sign-in state before a turn. Billing or rate-limit errors.

## Done

`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3.

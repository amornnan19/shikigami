# Plan: issue #19, recover from crashes, failures and stalls

## Goal

When something goes wrong (the app quit mid-turn, the provider died, the provider went silent, the
workspace folder vanished, the CLI disappeared), the user sees what happened and how to continue,
and nothing waits forever.

## Decisions

### Stall timeout (Rust, `src-tauri/src/turns/runner.rs`)

- A turn whose process writes no stdout line for 5 minutes is ended: kill its process group exactly
  like Stop (SIGTERM, SIGKILL after 2 s), status `failed`, error
  "No response from the provider for 5 minutes.". Running steps become `stopped`. It counts as a
  failure after spawn, so the queue advances.
- The timeout is a field of the runner's launch, default 300 s. Tests inject a short value. An
  `e2e`-feature-only environment variable `SHIKIGAMI_STALL_SECONDS` overrides it for e2e.
- A stdout line that the parser ignores still counts as activity.

### Agent attention reasons (Rust, `src-tauri/src/agents/`)

- `Agent` gains `attention: Option<{ code, message }>` (present exactly when `state` is
  `needs-attention`), checked in this order, first match wins:
  - `workspace-missing`: "The workspace folder is missing."
  - `provider-unavailable`: "Claude Code is not installed." (from the provider check)
  - `last-turn-interrupted`: "The app closed while the agent was working."
  - `last-turn-failed`: "The last reply failed: <turn error>"
- `recreate_workspace(agentId) -> Agent`: creates the folder (and its parents) if missing and
  writes the provider's instruction file with the default text only if the file is missing. Errors
  `{ code, message }`: `not-found`, `folder-failed`. It never touches an existing folder's content.

### Resume a waiting queue

- `resume_queue(agentId) -> ()`: if the agent has no running turn and its queue is not empty,
  start the head of the queue (same path as advancing). No-op otherwise. Refused while closing.
- The chat shows a bar above the composer when the queue is not empty and no turn runs:
  "N queued messages are waiting." with a "Resume" button. When the reason is an interrupted last
  turn, the text is "The app closed while the agent was working. N queued messages are waiting."

### UI

- An attention banner at the top of the chat for `workspace-missing` and `provider-unavailable`,
  with the message; `workspace-missing` has a "Recreate folder" button; `provider-unavailable` has a
  "Check again" button that refreshes providers and agents.
- While the workspace is missing or the provider is unavailable, the composer is disabled and its
  placeholder shows the reason.
- Interrupted and failed turns show their status line in the transcript (failed already shows its
  error; add "Interrupted" for interrupted turns).
- The agents query refetches when a `turn-event` `ended` arrives and when the window regains focus,
  so attention states stay current.

## Tests

- Rust: stall with a fake `claude` that prints one line then sleeps: ends `failed` with the message
  within the injected timeout plus grace, the group is gone, the queue advances; a slowly-but-steadily
  printing fake is not stalled.
- Rust: attention reasons for each case and their priority order; `recreate_workspace` creates the
  folder and instruction file, leaves an existing file untouched, refuses an unknown agent.
- Rust: `resume_queue` starts the head after an interrupted turn, is a no-op while a turn runs or the
  queue is empty, and is refused while closing.
- Rust: killing the provider process mid-turn marks it `failed` with its stderr line, and the next
  message works (spec check from the issue).
- E2E (fakes): delete the workspace folder from the test process, refocus or refresh: the banner and
  "Recreate folder" appear; recreate works and the composer is enabled again. After the restart spec
  (interrupted turn with a queued message), "Resume" starts the queued message. A stall with
  `SHIKIGAMI_STALL_SECONDS=3` shows the stall error. Screenshots of each.

## Manual check

None with the real CLI is needed; one `check-app` run with fakes is optional.

## Out of scope

Retrying failed turns automatically, backup models, editing agents (#20).

## Done

`bun run check`; `bun run e2e` 3/3; spec check 7 and the issue's checks pass.

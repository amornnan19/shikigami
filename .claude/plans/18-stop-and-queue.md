# Plan: issue #18, stop a turn and queue messages

## Goal

The user can stop a running turn and keep sending messages while the agent works. Messages sent
during a turn wait in a visible queue that can be edited or emptied, and the next one starts by
itself when the turn ends. The queue survives restarts.

## Decisions

### Stop (Rust, `src-tauri/src/turns/`)

- Command `stop_turn(agentId) -> ()`. If the agent has no running turn it does nothing (no error).
- It marks the running turn's runner as "stop requested" (a flag in `TurnRegistry` next to the
  process group id), sends SIGTERM to the group, waits up to 2 s, then SIGKILL. It returns after
  the signal is sent; the runner finishes the turn.
- The runner, when its process ends with the stop flag set: turn status `stopped` (not `failed`),
  steps still running become `stopped`, the partial reply text stays (flush the held-back text,
  redacted), `ended` event with status `stopped`. No resume retry after a stop.
- A stop requested before the process has registered (between `begin_turn` and `set_group`) must
  still stop it: the runner checks the flag right after registering.

### Queue

- Uses the existing `queued_messages` table (no migration). `position` is the next integer after
  the agent's current maximum.
- `send_message(agentId, text)` changes: when the agent has a running turn, the message is added to
  the queue instead of returning `busy`. Response `{ kind: "started", turnId } | { kind: "queued",
  queuedId }`. The `busy` error code is removed. Validation (`empty`, `too-long`) applies to queued
  messages too.
- `list_queue(agentId) -> QueuedMessage[] { id, text, position, createdAt }` in position order.
- `edit_queued(queuedId, text) -> QueuedMessage` (same validation; error `not-found` if it already
  started or was removed).
- `remove_queued(queuedId) -> ()` (no error if already gone).
- Advancing: when a turn ends as `completed`, `failed` or `stopped`, in one transaction take the
  lowest-position queued message for that agent, delete it, and begin its turn (same path as a
  direct send). Then start the runner. Interrupted turns (startup sweep) do not advance the queue;
  #19 adds Resume.
- Event `queue-changed { agentId }` after every enqueue, edit, remove and advance.
- Concurrency: enqueue, edit, remove and advance run under the database mutex; an edit or remove
  racing an advance sees `not-found` / no-op, never a half state.

### UI (`src/features/chat/`)

- While a turn runs, the Send button becomes a Stop button (square icon, label "Stop"); the
  textarea stays enabled and Enter adds to the queue. A small hint above the input reads
  "Messages you send now will wait until the agent finishes."
- Queue list between the transcript and the input, shown only when not empty: each row shows the
  text (two lines max, then "…"), an Edit button (turns the row into a textarea with Save and
  Cancel) and a Remove button (x, aria-label "Remove queued message"). Header "Queued (N)".
- After Stop, the turn shows "Stopped" in a muted line under it.
- Data through TanStack Query: `queue` query per agent, refetched on `queue-changed`.

## Tests

- Rust:
  - stop during a turn: with a fake `claude` that sleeps between lines, the turn ends `stopped`
    within 3 s, partial text kept, running steps `stopped`, no retry;
  - stop with no running turn is a no-op;
  - stop requested before the group registers still stops the process;
  - send while running enqueues; positions increase; validation applies;
  - edit and remove; edit after it started gives `not-found`;
  - advance after `completed`, `failed` and `stopped` starts the next queued message, and only one
    at a time; interrupted at startup does not advance;
  - the queue survives reopening the database.
- E2E (fake `claude` in slow mode when the message contains "SLOW"):
  - send a slow message, press Stop: "Stopped" appears within 3 s;
  - send a slow message, send two more: both appear under "Queued (2)"; edit one, remove the other;
    after the first turn ends, the edited one runs and the queue is empty;
  - screenshots of the running state with Stop, the queue, and the stopped turn.

## Manual check

One real turn with `claude-haiku-5-5` (`check-app`): ask for a long answer ("Count from 1 to 200,
one number per line"), press Stop after a few lines: the turn shows Stopped and the `claude`
process is gone (`ps` by the PID group). Then send one message while another runs and see it queue
and start.

## Out of scope

Steering a running turn, reordering the queue, stall timeout and Resume (#19).

## Done

`bun run check`; `bun run e2e` 3/3; spec checks 4 and 5 pass; the manual check.

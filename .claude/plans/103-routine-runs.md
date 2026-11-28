# Plan: issue #103, run routines on schedule and keep their history

## Goal

Start due routines while Shikigami is open, as normal messages in the agent's chat, record every run
and skip, and handle missed times after the app was closed or the Mac slept.

## Decisions

### Data (migration V12, Database rules)

```sql
CREATE TABLE routine_runs (
  id TEXT PRIMARY KEY,
  routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
  scheduled_for INTEGER,           -- the scheduled instant, NULL for Run now
  turn_id TEXT REFERENCES turns(id) ON DELETE SET NULL,
  queued_id TEXT,                  -- the queued message id while it waits
  kind TEXT NOT NULL CHECK (kind IN ('scheduled','manual','late')),
  outcome TEXT NOT NULL CHECK (outcome IN ('queued','running','completed','failed','stopped','interrupted','skipped','missed')),
  reason TEXT,
  created_at INTEGER NOT NULL,
  ended_at INTEGER
);
CREATE INDEX routine_runs_routine ON routine_runs(routine_id, created_at);
ALTER TABLE turns ADD COLUMN routine_id TEXT REFERENCES routines(id) ON DELETE SET NULL;
ALTER TABLE turns ADD COLUMN routine_name TEXT;
ALTER TABLE queued_messages ADD COLUMN routine_id TEXT REFERENCES routines(id) ON DELETE CASCADE;
ALTER TABLE queued_messages ADD COLUMN routine_name TEXT;
```
(The plan's SQL comments are not written into the migration.) Keep at most 20 runs per routine:
delete older ones on insert. `routine_name` is a snapshot so past turns keep their marker after the
routine is deleted.

### Scheduler (`src-tauri/src/routines/scheduler.rs`)

- One Tokio task started after the database opens. Loop: find the earliest `next_run_at` among
  active, not-done routines; sleep until then or at most 60 s (re-evaluate every minute so clock
  changes, wake and edits are picked up); on wake, process every routine with `next_run_at <= now`.
  Edits, pause, resume, delete and Run now notify the task (a `tokio::sync::Notify`) to re-plan.
- Clock source behind a trait (`Clock::now()`); e2e builds can set a fake clock with
  `SHIKIGAMI_E2E_CLOCK_FILE` (a file holding epoch ms the test rewrites; the task re-reads it each
  loop and the loop interval becomes 200 ms in e2e builds).
- Processing a due routine (in one DB transaction):
  - Lateness = now - next_run_at. Due on time if lateness < 2 minutes.
  - Repeating routine, on time: start a run. Late by 2 minutes or more: record one `skipped` run
    with reason "Shikigami was not running" (if the app started after that time) or "the Mac was
    asleep" (the loop noticed a wall-clock jump of more than 2 minutes beyond its sleep), then set
    `next_run_at` to the next time after now. Never replay more than once.
  - Once routine: on time, or late by less than 24 h → start a run (`late` kind when late, outcome
    shows "Ran late"); late by 24 h or more → record `missed`; either way set `done = 1`,
    `next_run_at = NULL`.
  - Overlap: if a previous run of the same routine is `queued` or `running`, record `skipped` with
    reason "the last run had not finished" and advance `next_run_at`.
- Starting a run = enqueue a message into the agent's one-to-one queue with `routine_id` and
  `routine_name` (reuse the existing send path: start now if idle, else queue), insert the run row
  (`queued` or `running`). When the turn starts, link `turn_id` and set `running`; when it ends, set
  the outcome from the turn status (`completed`, `failed`, `stopped`, `interrupted`) and
  `ended_at`. Queued run removed by the user → `skipped` "removed from the queue".
- No run starts while the registry is closing; the startup sweep marks `running` runs whose turn is
  interrupted as `interrupted`.
- Run now: `run_routine_now(routineId)` starts a `manual` run (also when paused or done), with the
  same overlap rule.
- An agent held (agent-level failure) or needing attention: the message queues as any message does;
  the run stays `queued` until it runs or is removed.

### UI

- Chat: a turn with `routineName` shows a small marker above the user bubble: "Routine: <name>"
  (with a clock icon).
- Routines list rows: last outcome ("Last run: Completed 08:00", "Skipped: the last run had not
  finished", "Ran late 09:14", "Missed") and a "Run now" button; a "History" toggle per routine
  showing the last 20 runs (time, kind, outcome, reason) each with "Open" scrolling the chat to its
  turn when it has one.
- Routine row "Done" once routines keep Run now.

## Tests

- Rust (fake clock): on-time run queues a message with the marker; busy agent queues it; overlap
  skip; once → Done; late repeating → one skip and next time; once late < 24 h runs as late, > 24 h
  missed; sleep jump detection; Run now when paused; run outcome follows the turn; history capped at
  20; routine deleted keeps the turn marker; closing registry starts nothing; migration V12 on V11.
- E2E (fake claude, fake clock file): spec checks 2, 3, 4, 6, 7 (simulated with clock jumps and a
  relaunch), 10, 11; screenshots.

## Done

`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3.

# Plan: issue #102, create and manage routines for an agent

## Goal

Store routines per agent and let the user create, edit, pause, resume and delete them, with the next
run time computed correctly in the Mac's time zone. Nothing runs yet (#103).

## Decisions

### Time handling

- Add `jiff` (exact pin, latest 0.x release; MIT OR Unlicense; check) for time zones and DST. Use the
  system time zone (`jiff::tz::TimeZone::system()`), with an override `SHIKIGAMI_TZ` (IANA name)
  honoured only in e2e and test builds. Rejected: chrono + chrono-tz (two crates, bundled tz data).
- Schedules store wall-clock fields, not instants; `next_run_at` (UTC milliseconds) is derived and
  recomputed at startup, on every edit, and when the system zone changes (the next-run computation
  takes the zone as a parameter; #103 calls it).
- DST: a wall time that does not exist runs at the first valid minute after it (jiff's
  "compatible" disambiguation forward); a repeated wall time runs at its first occurrence only.

### Data (migration V11, Database rules)

```sql
CREATE TABLE routines (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  name TEXT NOT NULL COLLATE NOCASE,
  task TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('once','daily','hourly','weekly')),
  at_time TEXT NOT NULL,            -- 'HH:MM', 24 h
  on_date TEXT,                     -- 'YYYY-MM-DD', once only
  days INTEGER NOT NULL DEFAULT 127,-- bitmask Mon=1 Tue=2 Wed=4 Thu=8 Fri=16 Sat=32 Sun=64 (daily, hourly)
  every_hours INTEGER,              -- 1-12, hourly only
  weekday INTEGER,                  -- 1=Mon..7=Sun, weekly only
  active INTEGER NOT NULL DEFAULT 1,
  done INTEGER NOT NULL DEFAULT 0,  -- once routines after their run (#103 sets it)
  next_run_at INTEGER,              -- NULL when paused or done
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (agent_id, name)
);
CREATE INDEX routines_due ON routines(next_run_at) WHERE next_run_at IS NOT NULL;
```
(No SQL comments in the migration file itself; the comments above are for this plan.)

### Rust (`src-tauri/src/routines/`)

- `schedule.rs` (pure): `Schedule` enum, `next_after(schedule, after: Timestamp, tz) ->
  Option<Timestamp>`, `describe(schedule) -> String`. Hourly: runs at `at_time + k*every_hours`
  within the same day (k >= 0, stopping before midnight), on chosen days.
- Wording of `describe`: "Once on 3 Dec 2026 at 08:00"; daily with all days "Every day at
  08:00", Mon-Fri "Every weekday at 08:00", Sat+Sun "Weekends at 08:00", otherwise "Every Mon, Wed,
  Fri at 08:00"; hourly "Every 3 hours from 09:00, every day" (day phrase as daily, lower case:
  "on weekdays", "on weekends", "on Mon, Wed"); weekly "Every Monday at 08:00".
- Commands: `list_routines(agentId) -> [Routine]`, `create_routine(agentId, input)`,
  `update_routine(routineId, input)`, `set_routine_active(routineId, active)`,
  `delete_routine(routineId)`. `Routine { id, agentId, name, task, schedule, description,
  active, done, nextRunAt, createdAt, updatedAt, lastRun: null }` (`lastRun` filled by #103).
- Validation (Rust is the authority), codes: `name-empty`, `name-too-long` (over 60 scalars),
  `name-invalid` (control characters), `name-taken`, `task-empty`, `task-too-long` (over 20,000),
  `time-invalid`, `date-invalid`, `date-past` (once in the past), `days-empty`, `hours-invalid`,
  `weekday-invalid`, `too-many-routines` (32 per agent), `agent-not-found`.
- A once routine in the past cannot be created; editing a done once routine to a future time makes
  it not done.
- Delete agent: cascade (already by FK); routine rows go.

### UI (`src/features/routines/`)

- Agent settings dialog gets a "Routines" section: list rows (name, description, "Next: <local date
  time>" or "Paused" or "Done"), a "New routine" button, and per row Edit, Pause/Resume, Delete
  (confirm "Delete <name>? Its run history is removed. Past messages stay in the chat.").
- Routine editor dialog: Name, Task (textarea with the hint "Each run starts from this message, so
  write it so it makes sense on its own."), Schedule kind (radio: Once, Every day, Every N hours,
  Every week) with its fields (date, time, day toggles Mon-Sun, hours select 1-12, weekday select),
  and the time zone shown read-only ("Times are in <zone name>.").
- When the agent's effective sign-in mode is subscription, the editor shows: "Routines run without
  you. Anthropic expects heavy automated use to use an API key. Consider switching this agent to an
  API key." with a link "Change sign-in" that opens the agent's sign-in setting. Saving is allowed.
- TanStack Query `["routines", agentId]`.

## Tests

- Rust: `next_after` across days, weekday masks, hourly windows, weekly, once; DST spring-forward
  (Europe/Berlin and America/New_York) and fall-back; `describe` strings; every validation code;
  32 limit; unique name per agent (another agent may reuse it); cascade on agent delete; migration
  V11 on a V10 database.
- E2E (`SHIKIGAMI_TZ`): create a daily routine and see its description and next run; pause shows
  Paused, resume shows the next run; edit; delete; the subscription notice; screenshots.

## Out of scope

Running routines and history (#103).

## Done

`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3.

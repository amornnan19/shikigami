# Spec: routines

- Milestone: M3 MVP: routines
- Status: draft

## Purpose

A routine is a task an agent does on its own on a schedule: every morning, every few hours, or once
at a set time. The user writes the task once, picks when it runs, and later reads the results in the
agent's chat. Routines run only while Shikigami is open, under the same sandbox and approval rules
as any other turn, and they prefer the user's API key because unattended work is heavy use.

## User stories

1. As a user, I can give an agent a routine with a name, a task and a schedule.
2. As a user, I can see an agent's routines, when each runs next, and what happened last time.
3. As a user, I can pause, resume, edit, run now and delete a routine.
4. As a user, I read each run's result in the agent's chat, marked as coming from the routine.
5. As a user, I know what happens when the Mac was asleep or Shikigami was closed at the scheduled
   time.
6. As a user, I know a routine cannot do anything a normal turn could not, and that risky steps
   still wait for me.

## Behaviour

### A routine

- Belongs to one agent. An agent can have up to 32 routines.
- Fields:
  - **Name**: 1-60 characters, unique among that agent's routines (case-insensitive).
  - **Task**: the message the agent receives on each run, 1-20,000 characters. The editor notes that
    each run should make sense on its own.
  - **Schedule**, in the Mac's current time zone, shown with the zone name:
    - **Once**: a date and a time.
    - **Every day**: a time, and the days of the week it runs (default all seven).
    - **Every N hours**: N from 1 to 12, starting at a chosen time, on chosen days.
    - **Every week**: a weekday and a time.
  - **Active** or **Paused**. A new routine is active.
- The list (in the agent's settings, a "Routines" section) shows each routine's name, its schedule
  in words ("Every weekday at 08:00"), the next run time, and the last run's outcome. A once
  routine that has run shows "Done" and has no next run.

### Runs

- At the scheduled time the routine's task becomes a message in the agent's own chat, marked
  "Routine: <name>", and runs like any other message: it waits in the queue if the agent is busy,
  and uses the agent's sign-in mode, sandbox and approval rules.
- **Run now** starts a run at once (queued if the agent is busy), even when the routine is paused.
- If the routine's previous run is still running or still waiting in the queue, the new run is
  skipped and recorded as "Skipped: the last run had not finished."
- Each run's outcome is recorded with the routine: started, completed, failed, stopped, interrupted
  or skipped (with the reason), and the time. The routine shows its last 20 runs; each links to its
  turn in the chat.
- Editing a routine changes future runs only. Pausing it does not stop a run already started.

### Missed runs

- Routines run only while Shikigami is open and the Mac is awake.
- If Shikigami was closed or the Mac was asleep at a scheduled time:
  - a repeating routine skips the missed times and records one "Skipped: Shikigami was not running"
    (or "the Mac was asleep") entry, then continues at its next time;
  - a once routine that was missed by less than 24 hours runs once when Shikigami opens or the Mac
    wakes, recorded as "Ran late"; missed by more, it is recorded as skipped and shows "Missed".
- A change of the Mac's time zone keeps each routine's clock time in the new zone.

### Approvals with nobody present

- A routine run follows the same rules as any turn: a request that asks first shows an approval
  card in the agent's chat, the agent's row shows it needs the user, and no answer within 10 minutes
  means deny. A routine never allows anything by itself.
- Work a routine causes another agent to do runs at the lower of the two access levels (all agents
  share one level for now).

### Sign-in for unattended work

- When the user creates a routine for an agent that uses its Claude subscription, the editor shows
  a notice: "Routines run without you. Anthropic expects heavy automated use to use an API key.
  Consider switching this agent to an API key." with a link to the agent's sign-in setting. It does
  not block saving.
- A run whose sign-in fails ends failed and holds the agent like any other sign-in failure.

### Deleting

- Deleting a routine removes it and its run history; past turns stay in the chat with their marker.
  A run already started continues; a queued run is removed from the queue.
- Deleting an agent deletes its routines.

## Edge cases

- The agent's folder is missing or the agent needs attention: the run fails (or is held) as a
  normal turn would, and is recorded.
- Shikigami is shutting down: no run starts.
- Many routines due at the same moment: each is queued in order of its scheduled time, then name.
- A schedule time that does not exist on a day (a daylight-saving jump) runs at the next valid
  minute; a repeated hour runs once.
- The system clock moves backwards: no run repeats a time it already ran.

## Safety

- Routines never widen what an agent can do: same sandbox, same approvals, same refusals.
- No answer to an approval means deny, including in routine runs.
- Nothing runs while Shikigami is closed.

## Acceptance checks

1. Create a daily routine for an agent; the list shows its schedule in words and the next run time.
2. At its time (testable with a fast clock in test builds) the task appears in the agent's chat
   marked "Routine: <name>" and runs.
3. Run now starts a run immediately, also when paused.
4. A routine whose previous run is still going records "Skipped: the last run had not finished."
5. Pause stops future runs; resume brings them back with the right next time.
6. A once routine runs once and then shows Done.
7. With Shikigami closed over a scheduled time, reopening records the skip for a repeating routine
   and runs a once routine missed by less than 24 hours.
8. A routine run that needs approval shows a card; no answer in 10 minutes denies it.
9. Creating a routine for a subscription-mode agent shows the API-key notice.
10. The routine's history lists its last runs with outcomes, each opening the turn.
11. Deleting an agent removes its routines; deleting a routine keeps past turns in the chat.

## Out of scope

- Webhooks, script triggers and custom cron expressions.
- Routines for group chats.
- Sending results elsewhere (messaging apps, notifications beyond the app).
- Calendar views and feeds.
- Keeping the Mac awake.

import { describe, expect, it } from "vitest";
import type { Routine, RoutineRun, Schedule } from "./api";
import { formOf, inputOf, lastRunOf, outcomeOf, scheduleOf, statusOf } from "./form";

function routine(patch: Partial<Routine>, schedule: Schedule): Routine {
  return {
    id: "r",
    agentId: "a",
    name: "Morning",
    task: "Check",
    schedule,
    description: "",
    active: true,
    done: false,
    nextRunAt: 1,
    nextRunText: "Sun 29 Nov 2026, 08:00",
    createdAt: 1,
    updatedAt: 1,
    lastRun: null,
    ...patch,
  };
}

describe("routine form", () => {
  it("round-trips every kind of schedule", () => {
    const schedules: Schedule[] = [
      { kind: "once", date: "2026-12-03", time: "08:00" },
      { kind: "daily", time: "08:30", days: 31 },
      { kind: "hourly", time: "09:05", everyHours: 3, days: 96 },
      { kind: "weekly", weekday: 7, time: "23:59" },
    ];
    for (const schedule of schedules) {
      expect(scheduleOf(formOf(routine({}, schedule)))).toEqual(schedule);
    }
  });

  it("sends the name, task and schedule as typed", () => {
    const form = formOf(
      routine({ name: " Spaced ", task: "Do it" }, { kind: "daily", time: "08:00", days: 127 }),
    );
    expect(inputOf(form)).toEqual({
      name: " Spaced ",
      task: "Do it",
      schedule: { kind: "daily", time: "08:00", days: 127 },
    });
  });

  it("shows done before paused before the next run", () => {
    const schedule: Schedule = { kind: "daily", time: "08:00", days: 127 };
    expect(statusOf(routine({}, schedule))).toBe("Next: Sun 29 Nov 2026, 08:00");
    expect(statusOf(routine({ active: false, nextRunText: null }, schedule))).toBe("Paused");
    expect(statusOf(routine({ done: true, active: false }, schedule))).toBe("Done");
    expect(statusOf(routine({ nextRunText: null }, schedule))).toBe("No upcoming run");
  });

  it("describes how a run went, with the reason for a skip", () => {
    const run = (patch: Partial<RoutineRun>): RoutineRun => ({
      id: "u",
      routineId: "r",
      scheduledFor: 1,
      turnId: null,
      kind: "scheduled",
      outcome: "completed",
      reason: null,
      createdAt: 1,
      endedAt: 2,
      timeText: "08:00",
      ...patch,
    });
    expect(outcomeOf(run({}))).toBe("Completed");
    expect(outcomeOf(run({ outcome: "interrupted" }))).toBe("Interrupted");
    expect(outcomeOf(run({ outcome: "queued" }))).toBe("Waiting in the queue");
    expect(outcomeOf(run({ outcome: "skipped", reason: "the last run had not finished" }))).toBe(
      "Skipped: the last run had not finished.",
    );
    expect(outcomeOf(run({ outcome: "missed" }))).toBe("Missed");
    expect(outcomeOf(run({ kind: "late" }))).toBe("Ran late");
    expect(outcomeOf(run({ kind: "late", outcome: "failed" }))).toBe("Ran late, failed");
  });

  it("shows the last run with its time, except for skips and misses", () => {
    const schedule: Schedule = { kind: "daily", time: "08:00", days: 127 };
    const last = (patch: Partial<RoutineRun>) =>
      routine(
        {
          lastRun: {
            id: "u",
            routineId: "r",
            scheduledFor: 1,
            turnId: null,
            kind: "scheduled",
            outcome: "completed",
            reason: null,
            createdAt: 1,
            endedAt: 2,
            timeText: "08:00",
            ...patch,
          },
        },
        schedule,
      );
    expect(lastRunOf(routine({}, schedule))).toBeNull();
    expect(lastRunOf(last({}))).toBe("Last run: Completed 08:00");
    expect(lastRunOf(last({ kind: "late", timeText: "09:14" }))).toBe("Last run: Ran late 09:14");
    expect(lastRunOf(last({ outcome: "missed" }))).toBe("Last run: Missed");
    expect(lastRunOf(last({ outcome: "skipped", reason: "the Mac was asleep" }))).toBe(
      "Last run: Skipped: the Mac was asleep.",
    );
  });
});

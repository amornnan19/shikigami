import type { Routine, RoutineInput, RoutineRun, Schedule } from "./api";

export type Kind = Schedule["kind"];

export const ALL_DAYS = 127;
export const DAY_SHORT = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
export const DAY_LONG = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];
export const KIND_LABELS: Record<Kind, string> = {
  once: "Once",
  daily: "Every day",
  hourly: "Every N hours",
  weekly: "Every week",
};

export interface RoutineForm {
  name: string;
  task: string;
  kind: Kind;
  date: string;
  time: string;
  days: number;
  everyHours: number;
  weekday: number;
}

function tomorrow() {
  const next = new Date();
  next.setDate(next.getDate() + 1);
  const month = String(next.getMonth() + 1).padStart(2, "0");
  const day = String(next.getDate()).padStart(2, "0");
  return `${next.getFullYear()}-${month}-${day}`;
}

export function emptyForm(): RoutineForm {
  return {
    name: "",
    task: "",
    kind: "daily",
    date: tomorrow(),
    time: "08:00",
    days: ALL_DAYS,
    everyHours: 3,
    weekday: 1,
  };
}

export function formOf(routine: Routine): RoutineForm {
  const form = { ...emptyForm(), name: routine.name, task: routine.task };
  const schedule = routine.schedule;
  form.kind = schedule.kind;
  form.time = schedule.time;
  if (schedule.kind === "once") form.date = schedule.date;
  if (schedule.kind === "daily" || schedule.kind === "hourly") form.days = schedule.days;
  if (schedule.kind === "hourly") form.everyHours = schedule.everyHours;
  if (schedule.kind === "weekly") form.weekday = schedule.weekday;
  return form;
}

export function scheduleOf(form: RoutineForm): Schedule {
  switch (form.kind) {
    case "once":
      return { kind: "once", date: form.date, time: form.time };
    case "daily":
      return { kind: "daily", time: form.time, days: form.days };
    case "hourly":
      return { kind: "hourly", time: form.time, everyHours: form.everyHours, days: form.days };
    case "weekly":
      return { kind: "weekly", weekday: form.weekday, time: form.time };
  }
}

export function inputOf(form: RoutineForm): RoutineInput {
  return { name: form.name, task: form.task, schedule: scheduleOf(form) };
}

export function statusOf(routine: Routine) {
  if (routine.done) return "Done";
  if (!routine.active) return "Paused";
  return routine.nextRunText ? `Next: ${routine.nextRunText}` : "No upcoming run";
}

const OUTCOME_TEXT = {
  queued: "Waiting in the queue",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  stopped: "Stopped",
  interrupted: "Interrupted",
  missed: "Missed",
};

export const KIND_TEXT = { scheduled: "Scheduled", manual: "Run now", late: "Ran late" };

export function outcomeOf(run: RoutineRun) {
  if (run.outcome === "skipped") return `Skipped: ${run.reason ?? "no reason recorded"}.`;
  const text = OUTCOME_TEXT[run.outcome];
  if (run.kind !== "late") return text;
  return run.outcome === "completed" || run.outcome === "running" || run.outcome === "queued"
    ? "Ran late"
    : `Ran late, ${text.toLowerCase()}`;
}

export function lastRunOf(routine: Routine) {
  const run = routine.lastRun;
  if (!run) return null;
  const withTime = run.outcome !== "skipped" && run.outcome !== "missed";
  return `Last run: ${outcomeOf(run)}${withTime ? ` ${run.timeText}` : ""}`;
}

export const NAME_CODES = ["name-empty", "name-too-long", "name-invalid", "name-taken"];
export const TASK_CODES = ["task-empty", "task-too-long"];
export const DATE_CODES = ["date-invalid", "date-past"];
export const TIME_CODES = ["time-invalid"];
export const DAYS_CODES = ["days-empty"];
export const HOURS_CODES = ["hours-invalid"];
export const WEEKDAY_CODES = ["weekday-invalid"];
export const FIELD_CODES = [
  ...NAME_CODES,
  ...TASK_CODES,
  ...DATE_CODES,
  ...TIME_CODES,
  ...DAYS_CODES,
  ...HOURS_CODES,
  ...WEEKDAY_CODES,
];

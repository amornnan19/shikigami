import { invoke } from "@tauri-apps/api/core";

export type Schedule =
  | { kind: "once"; date: string; time: string }
  | { kind: "daily"; time: string; days: number }
  | { kind: "hourly"; time: string; everyHours: number; days: number }
  | { kind: "weekly"; weekday: number; time: string };

export type RunKind = "scheduled" | "manual" | "late";

export type RunOutcome =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "stopped"
  | "interrupted"
  | "skipped"
  | "missed";

export interface RoutineRun {
  id: string;
  routineId: string;
  scheduledFor: number | null;
  turnId: string | null;
  kind: RunKind;
  outcome: RunOutcome;
  reason: string | null;
  createdAt: number;
  endedAt: number | null;
  timeText: string;
}

export interface Routine {
  id: string;
  agentId: string;
  name: string;
  task: string;
  schedule: Schedule;
  description: string;
  active: boolean;
  done: boolean;
  nextRunAt: number | null;
  nextRunText: string | null;
  createdAt: number;
  updatedAt: number;
  lastRun: RoutineRun | null;
}

export interface RoutineInput {
  name: string;
  task: string;
  schedule: Schedule;
}

export const routinesKey = (agentId: string) => ["routines", agentId];
export const runsKey = (routineId: string) => ["routine-runs", routineId];
export const TIME_ZONE_KEY = ["routine-time-zone"];

export function listRoutines(agentId: string) {
  return invoke<Routine[]>("list_routines", { agentId });
}

export function createRoutine(agentId: string, input: RoutineInput) {
  return invoke<Routine>("create_routine", { agentId, input });
}

export function updateRoutine(routineId: string, input: RoutineInput) {
  return invoke<Routine>("update_routine", { routineId, input });
}

export function setRoutineActive(routineId: string, active: boolean) {
  return invoke<Routine>("set_routine_active", { routineId, active });
}

export function deleteRoutine(routineId: string) {
  return invoke<void>("delete_routine", { routineId });
}

export function getRoutineTimeZone() {
  return invoke<string>("routine_time_zone");
}

export function listRoutineRuns(routineId: string) {
  return invoke<RoutineRun[]>("list_routine_runs", { routineId });
}

export function runRoutineNow(routineId: string) {
  return invoke<RoutineRun | null>("run_routine_now", { routineId });
}

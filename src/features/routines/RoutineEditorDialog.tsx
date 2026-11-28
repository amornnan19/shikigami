import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { errorFor, FieldError } from "@/features/agents/AgentFields";
import { type AgentError, asAgentError } from "@/features/agents/api";
import { cn } from "@/lib/utils";
import {
  createRoutine,
  getRoutineTimeZone,
  type Routine,
  routinesKey,
  TIME_ZONE_KEY,
  updateRoutine,
} from "./api";
import {
  DATE_CODES,
  DAY_LONG,
  DAY_SHORT,
  DAYS_CODES,
  emptyForm,
  FIELD_CODES,
  formOf,
  HOURS_CODES,
  inputOf,
  KIND_LABELS,
  type Kind,
  NAME_CODES,
  type RoutineForm,
  TASK_CODES,
  TIME_CODES,
  WEEKDAY_CODES,
} from "./form";

const KINDS: Kind[] = ["once", "daily", "hourly", "weekly"];
const HOURS = Array.from({ length: 12 }, (_, index) => index + 1);

function DayToggles({
  days,
  onChange,
  error,
}: {
  days: number;
  onChange: (days: number) => void;
  error: string | null;
}) {
  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="mb-2 text-sm font-medium">Days</legend>
      <div className="flex flex-wrap gap-1.5">
        {DAY_SHORT.map((short, index) => {
          const bit = 1 << index;
          const on = (days & bit) !== 0;
          return (
            <button
              key={short}
              type="button"
              aria-pressed={on}
              aria-label={DAY_LONG[index]}
              onClick={() => onChange(days ^ bit)}
              className={cn(
                "h-8 min-w-11 rounded-md border border-border px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring",
                on ? "bg-foreground text-background" : "bg-background hover:bg-accent",
              )}
            >
              {short}
            </button>
          );
        })}
      </div>
      <FieldError id="routine-days-error" message={error} />
    </fieldset>
  );
}

function TimeField({
  id,
  label,
  value,
  onChange,
  error,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  error: string | null;
}) {
  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="time"
        value={value}
        aria-invalid={error !== null}
        aria-describedby={`${id}-error`}
        onChange={(event) => onChange(event.target.value)}
      />
      <FieldError id={`${id}-error`} message={error} />
    </div>
  );
}

function ScheduleFields({
  form,
  set,
  error,
}: {
  form: RoutineForm;
  set: (patch: Partial<RoutineForm>) => void;
  error: AgentError | null;
}) {
  const timeError = errorFor(error, TIME_CODES);
  const daysError = errorFor(error, DAYS_CODES);
  return (
    <>
      {form.kind === "once" && (
        <div className="flex flex-col gap-2">
          <Label htmlFor="routine-date">Date</Label>
          <Input
            id="routine-date"
            type="date"
            value={form.date}
            aria-invalid={errorFor(error, DATE_CODES) !== null}
            aria-describedby="routine-date-error"
            onChange={(event) => set({ date: event.target.value })}
          />
          <FieldError id="routine-date-error" message={errorFor(error, DATE_CODES)} />
        </div>
      )}
      {form.kind === "weekly" && (
        <div className="flex flex-col gap-2">
          <Label htmlFor="routine-weekday">Day of the week</Label>
          <Select
            value={String(form.weekday)}
            onValueChange={(value) => set({ weekday: Number(value) })}
          >
            <SelectTrigger id="routine-weekday">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {DAY_LONG.map((day, index) => (
                <SelectItem key={day} value={String(index + 1)}>
                  {day}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <FieldError id="routine-weekday-error" message={errorFor(error, WEEKDAY_CODES)} />
        </div>
      )}
      {form.kind === "hourly" && (
        <div className="flex flex-col gap-2">
          <Label htmlFor="routine-hours">Every</Label>
          <Select
            value={String(form.everyHours)}
            onValueChange={(value) => set({ everyHours: Number(value) })}
          >
            <SelectTrigger id="routine-hours">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {HOURS.map((hours) => (
                <SelectItem key={hours} value={String(hours)}>
                  {hours === 1 ? "1 hour" : `${hours} hours`}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <FieldError id="routine-hours-error" message={errorFor(error, HOURS_CODES)} />
        </div>
      )}
      <TimeField
        id="routine-time"
        label={form.kind === "hourly" ? "Starting at" : "At"}
        value={form.time}
        onChange={(time) => set({ time })}
        error={timeError}
      />
      {(form.kind === "daily" || form.kind === "hourly") && (
        <DayToggles days={form.days} onChange={(days) => set({ days })} error={daysError} />
      )}
    </>
  );
}

export function RoutineEditorDialog({
  agentId,
  routine,
  subscription,
  open,
  onOpenChange,
  onChangeSignIn,
}: {
  agentId: string;
  routine: Routine | null;
  subscription: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChangeSignIn: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        aria-describedby="routine-editor-description"
        className="max-h-[90vh] max-w-lg overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>{routine ? "Edit routine" : "New routine"}</DialogTitle>
          <DialogDescription id="routine-editor-description">
            A routine sends its task to the agent on a schedule.
          </DialogDescription>
        </DialogHeader>
        <EditorForm
          agentId={agentId}
          routine={routine}
          subscription={subscription}
          onDone={() => onOpenChange(false)}
          onChangeSignIn={onChangeSignIn}
        />
      </DialogContent>
    </Dialog>
  );
}

function EditorForm({
  agentId,
  routine,
  subscription,
  onDone,
  onChangeSignIn,
}: {
  agentId: string;
  routine: Routine | null;
  subscription: boolean;
  onDone: () => void;
  onChangeSignIn: () => void;
}) {
  const client = useQueryClient();
  const [form, setForm] = useState(() => (routine ? formOf(routine) : emptyForm()));
  const [error, setError] = useState<AgentError | null>(null);
  const zone = useQuery({ queryKey: TIME_ZONE_KEY, queryFn: getRoutineTimeZone, retry: false });
  const save = useMutation({
    mutationFn: () =>
      routine ? updateRoutine(routine.id, inputOf(form)) : createRoutine(agentId, inputOf(form)),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: routinesKey(agentId) });
      onDone();
    },
    onError: (failure) => setError(asAgentError(failure)),
  });
  const set = (patch: Partial<RoutineForm>) => {
    setForm((current) => ({ ...current, ...patch }));
    setError(null);
  };
  const nameError = errorFor(error, NAME_CODES);
  const taskError = errorFor(error, TASK_CODES);
  const otherError = error && !FIELD_CODES.includes(error.code) ? error.message : null;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-2">
        <Label htmlFor="routine-name">Name</Label>
        <Input
          id="routine-name"
          value={form.name}
          autoComplete="off"
          aria-invalid={nameError !== null}
          aria-describedby="routine-name-error"
          onChange={(event) => set({ name: event.target.value })}
        />
        <FieldError id="routine-name-error" message={nameError} />
      </div>
      <div className="flex flex-col gap-2">
        <Label htmlFor="routine-task">Task</Label>
        <textarea
          id="routine-task"
          value={form.task}
          rows={5}
          aria-invalid={taskError !== null}
          aria-describedby="routine-task-hint routine-task-error"
          onChange={(event) => set({ task: event.target.value })}
          className="w-full resize-y rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring aria-invalid:border-red-500"
        />
        <p id="routine-task-hint" className="text-sm text-muted-foreground">
          Each run starts from this message, so write it so it makes sense on its own.
        </p>
        <FieldError id="routine-task-error" message={taskError} />
      </div>
      <fieldset className="flex flex-col gap-2">
        <legend className="mb-2 text-sm font-medium">Schedule</legend>
        <div className="flex flex-wrap gap-x-4 gap-y-2">
          {KINDS.map((kind) => (
            <label key={kind} className="flex items-center gap-2 text-sm">
              <input
                type="radio"
                name="routine-kind"
                value={kind}
                checked={form.kind === kind}
                onChange={() => set({ kind })}
                className="size-4"
              />
              {KIND_LABELS[kind]}
            </label>
          ))}
        </div>
      </fieldset>
      <ScheduleFields form={form} set={set} error={error} />
      <p id="routine-time-zone" className="text-sm text-muted-foreground">
        {zone.data ? `Times are in ${zone.data}.` : "Times are in your Mac's time zone."}
      </p>
      {subscription && (
        <p
          id="routine-api-key-notice"
          role="note"
          className="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900"
        >
          Routines run without you. Anthropic expects heavy automated use to use an API key.
          Consider switching this agent to an API key.{" "}
          <button type="button" onClick={onChangeSignIn} className="font-medium underline">
            Change sign-in
          </button>
        </p>
      )}
      <FieldError id="routine-form-error" message={otherError} />
      <DialogFooter>
        <Button type="button" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button type="button" disabled={save.isPending} onClick={() => save.mutate()}>
          Save routine
        </Button>
      </DialogFooter>
    </div>
  );
}

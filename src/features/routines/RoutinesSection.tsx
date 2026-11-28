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
import { type Agent, asAgentError } from "@/features/agents/api";
import {
  deleteRoutine,
  listRoutineRuns,
  listRoutines,
  type Routine,
  type RoutineRun,
  routinesKey,
  runRoutineNow,
  runsKey,
  setRoutineActive,
} from "./api";
import { KIND_TEXT, lastRunOf, outcomeOf, statusOf } from "./form";
import { RoutineEditorDialog } from "./RoutineEditorDialog";

function DeleteRoutineDialog({
  routine,
  onOpenChange,
  onDeleted,
}: {
  routine: Routine | null;
  onOpenChange: (open: boolean) => void;
  onDeleted: () => void;
}) {
  const remove = useMutation({
    mutationFn: () => deleteRoutine(routine?.id ?? ""),
    onSuccess: onDeleted,
  });
  return (
    <Dialog
      open={routine !== null}
      onOpenChange={(open) => {
        if (!open) remove.reset();
        onOpenChange(open);
      }}
    >
      <DialogContent aria-describedby="delete-routine-description">
        <DialogHeader>
          <DialogTitle>Delete {routine?.name}?</DialogTitle>
          <DialogDescription id="delete-routine-description">
            Its run history is removed. Past messages stay in the chat.
          </DialogDescription>
        </DialogHeader>
        {remove.isError && (
          <p role="alert" className="text-sm text-red-600">
            {asAgentError(remove.error).message}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="destructive"
            disabled={remove.isPending}
            onClick={() => remove.mutate()}
          >
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

const REFRESH_MS = 1500;

function History({ routine, onOpenTurn }: { routine: Routine; onOpenTurn: (id: string) => void }) {
  const runs = useQuery({
    queryKey: runsKey(routine.id),
    queryFn: () => listRoutineRuns(routine.id),
    refetchInterval: REFRESH_MS,
  });
  if (runs.data?.length === 0) {
    return <p className="text-sm text-muted-foreground">No runs yet.</p>;
  }
  return (
    <ul data-part="history" className="flex flex-col gap-1 border-t border-border pt-2">
      {runs.data?.map((run: RoutineRun) => (
        <li
          key={run.id}
          data-run={run.outcome}
          className="flex items-center justify-between gap-3 text-sm"
        >
          <span className="min-w-0">
            <span className="text-muted-foreground">{run.timeText}</span>{" "}
            <span className="text-muted-foreground">{KIND_TEXT[run.kind]}</span>{" "}
            <span data-part="outcome">{outcomeOf(run)}</span>
          </span>
          {run.turnId && (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => onOpenTurn(run.turnId ?? "")}
            >
              Open
            </Button>
          )}
        </li>
      ))}
    </ul>
  );
}

function RoutineRow({
  routine,
  onEdit,
  onDelete,
  onFailed,
  onOpenTurn,
}: {
  routine: Routine;
  onEdit: () => void;
  onDelete: () => void;
  onFailed: (message: string | null) => void;
  onOpenTurn: (id: string) => void;
}) {
  const client = useQueryClient();
  const [showHistory, setShowHistory] = useState(false);
  const runNow = useMutation({
    mutationFn: () => runRoutineNow(routine.id),
    onSuccess: () => {
      onFailed(null);
      void client.invalidateQueries({ queryKey: runsKey(routine.id) });
      return client.invalidateQueries({ queryKey: routinesKey(routine.agentId) });
    },
    onError: (failure) => onFailed(asAgentError(failure).message),
  });
  const lastRun = lastRunOf(routine);
  const toggle = useMutation({
    mutationFn: () => setRoutineActive(routine.id, !routine.active),
    onSuccess: () => {
      onFailed(null);
      return client.invalidateQueries({ queryKey: routinesKey(routine.agentId) });
    },
    onError: (failure) => onFailed(asAgentError(failure).message),
  });
  return (
    <li
      data-routine={routine.name}
      className="flex flex-col gap-2 rounded-md border border-border px-3 py-2"
    >
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-medium">{routine.name}</span>
          <span data-part="description" className="text-sm text-muted-foreground">
            {routine.description}
          </span>
          <span data-part="status" className="text-sm text-muted-foreground">
            {statusOf(routine)}
          </span>
          {lastRun && (
            <span data-part="last-run" className="text-sm text-muted-foreground">
              {lastRun}
            </span>
          )}
        </div>
        <div className="flex shrink-0 gap-1">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={runNow.isPending}
            onClick={() => runNow.mutate()}
          >
            Run now
          </Button>
          <Button type="button" size="sm" variant="outline" onClick={onEdit}>
            Edit
          </Button>
          {!routine.done && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={toggle.isPending}
              onClick={() => toggle.mutate()}
            >
              {routine.active ? "Pause" : "Resume"}
            </Button>
          )}
          <Button
            type="button"
            size="sm"
            variant="outline"
            aria-expanded={showHistory}
            onClick={() => setShowHistory(!showHistory)}
          >
            History
          </Button>
          <Button type="button" size="sm" variant="outline" onClick={onDelete}>
            Delete
          </Button>
        </div>
      </div>
      {showHistory && <History routine={routine} onOpenTurn={onOpenTurn} />}
    </li>
  );
}

export function RoutinesSection({
  agent,
  onChangeSignIn,
  onOpenTurn,
}: {
  agent: Agent;
  onChangeSignIn: () => void;
  onOpenTurn: (turnId: string) => void;
}) {
  const client = useQueryClient();
  const routines = useQuery({
    queryKey: routinesKey(agent.id),
    queryFn: () => listRoutines(agent.id),
    refetchInterval: REFRESH_MS,
  });
  const [editing, setEditing] = useState<Routine | "new" | null>(null);
  const [deleting, setDeleting] = useState<Routine | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  return (
    <>
      {routines.isError && (
        <p role="alert" className="text-sm text-red-600">
          {asAgentError(routines.error).message}
        </p>
      )}
      {routines.data?.length === 0 && (
        <p className="text-sm text-muted-foreground">This agent has no routines yet.</p>
      )}
      <ul className="flex flex-col gap-2">
        {routines.data?.map((routine) => (
          <RoutineRow
            key={routine.id}
            routine={routine}
            onEdit={() => setEditing(routine)}
            onDelete={() => setDeleting(routine)}
            onFailed={setFailure}
            onOpenTurn={onOpenTurn}
          />
        ))}
      </ul>
      {failure && (
        <p role="alert" className="text-sm text-red-600">
          {failure}
        </p>
      )}
      <div>
        <Button type="button" size="sm" onClick={() => setEditing("new")}>
          New routine
        </Button>
      </div>
      {editing !== null && (
        <RoutineEditorDialog
          key={editing === "new" ? "new" : editing.id}
          agentId={agent.id}
          routine={editing === "new" ? null : editing}
          subscription={agent.effectiveAuthMode === "subscription"}
          open
          onOpenChange={(open) => {
            if (!open) setEditing(null);
          }}
          onChangeSignIn={() => {
            setEditing(null);
            onChangeSignIn();
          }}
        />
      )}
      <DeleteRoutineDialog
        routine={deleting}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
        onDeleted={() => {
          setDeleting(null);
          return client.invalidateQueries({ queryKey: routinesKey(agent.id) });
        }}
      />
    </>
  );
}

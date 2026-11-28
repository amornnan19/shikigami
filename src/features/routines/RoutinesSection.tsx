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
import { deleteRoutine, listRoutines, type Routine, routinesKey, setRoutineActive } from "./api";
import { statusOf } from "./form";
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

function RoutineRow({
  routine,
  onEdit,
  onDelete,
  onFailed,
}: {
  routine: Routine;
  onEdit: () => void;
  onDelete: () => void;
  onFailed: (message: string | null) => void;
}) {
  const client = useQueryClient();
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
      className="flex items-center justify-between gap-3 rounded-md border border-border px-3 py-2"
    >
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-sm font-medium">{routine.name}</span>
        <span data-part="description" className="text-sm text-muted-foreground">
          {routine.description}
        </span>
        <span data-part="status" className="text-sm text-muted-foreground">
          {statusOf(routine)}
        </span>
      </div>
      <div className="flex shrink-0 gap-1">
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
        <Button type="button" size="sm" variant="outline" onClick={onDelete}>
          Delete
        </Button>
      </div>
    </li>
  );
}

export function RoutinesSection({
  agent,
  onChangeSignIn,
}: {
  agent: Agent;
  onChangeSignIn: () => void;
}) {
  const client = useQueryClient();
  const routines = useQuery({
    queryKey: routinesKey(agent.id),
    queryFn: () => listRoutines(agent.id),
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

import { useMutation, useQueryClient } from "@tanstack/react-query";
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
import { type Agent, asAgentError, type Deleted, deleteAgent } from "./api";

function DeleteForm({
  agent,
  onCancel,
  onDeleted,
}: {
  agent: Agent;
  onCancel: () => void;
  onDeleted: (outcome: Deleted, askedForTrash: boolean) => void;
}) {
  const client = useQueryClient();
  const [moveToTrash, setMoveToTrash] = useState(false);
  const remove = useMutation({
    mutationFn: () => deleteAgent(agent.id, moveToTrash),
    onSuccess: async (outcome) => {
      client.removeQueries({ queryKey: ["conversation", agent.id] });
      client.removeQueries({ queryKey: ["queue", agent.id] });
      await client.invalidateQueries({ queryKey: ["agents"] });
      onDeleted(outcome, moveToTrash);
    },
    onError: () => client.invalidateQueries({ queryKey: ["agents"] }),
  });

  return (
    <div className="flex flex-col gap-4">
      <label className="flex items-start gap-2 text-sm">
        <input
          type="checkbox"
          checked={moveToTrash}
          onChange={(event) => setMoveToTrash(event.target.checked)}
          className="mt-0.5 size-4"
        />
        <span className="min-w-0 break-words">
          Also move the folder <span className="font-mono">{agent.workspacePath}</span> to the Trash
        </span>
      </label>
      {remove.isError && (
        <p role="alert" className="text-sm text-red-600">
          {asAgentError(remove.error).message}
        </p>
      )}
      <DialogFooter>
        <Button type="button" variant="ghost" onClick={onCancel}>
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
    </div>
  );
}

export function DeleteAgentDialog({
  agent,
  open,
  onOpenChange,
  onDeleted,
}: {
  agent: Agent;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDeleted: (outcome: Deleted, askedForTrash: boolean) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent aria-describedby="delete-agent-description">
        <DialogHeader>
          <DialogTitle>Delete {agent.name}?</DialogTitle>
          <DialogDescription id="delete-agent-description">
            Its conversation is removed from Shikigami.
          </DialogDescription>
        </DialogHeader>
        <DeleteForm agent={agent} onCancel={() => onOpenChange(false)} onDeleted={onDeleted} />
      </DialogContent>
    </Dialog>
  );
}

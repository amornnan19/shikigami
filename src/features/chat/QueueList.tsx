import { useMutation, useQueryClient } from "@tanstack/react-query";
import { X } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { asAgentError } from "@/features/agents/api";
import { editQueued, MESSAGE_MAX_CHARS, type QueuedMessage, removeQueued } from "./api";

function QueueRow({ message, agentId }: { message: QueuedMessage; agentId: string }) {
  const client = useQueryClient();
  const [draft, setDraft] = useState<string | null>(null);
  const refresh = () => client.invalidateQueries({ queryKey: ["queue", agentId] });
  const save = useMutation({
    mutationFn: (text: string) => editQueued(message.id, text),
    onSuccess: () => {
      setDraft(null);
      void refresh();
    },
    onError: () => void refresh(),
  });
  const remove = useMutation({
    mutationFn: () => removeQueued(message.id),
    onSuccess: () => void refresh(),
  });
  const valid =
    draft !== null && draft.trim().length > 0 && Array.from(draft).length <= MESSAGE_MAX_CHARS;

  if (draft !== null) {
    return (
      <li className="flex flex-col gap-2">
        <textarea
          aria-label="Edit queued message"
          rows={2}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          className="min-h-14 resize-y rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        {save.isError && (
          <p role="alert" className="text-sm text-red-600">
            {asAgentError(save.error).message}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" size="sm" onClick={() => setDraft(null)}>
            Cancel
          </Button>
          <Button type="button" size="sm" disabled={!valid} onClick={() => save.mutate(draft)}>
            Save
          </Button>
        </div>
      </li>
    );
  }

  return (
    <li className="flex items-start gap-2">
      <p className="line-clamp-2 min-w-0 flex-1 text-sm break-words whitespace-pre-wrap">
        {message.text}
      </p>
      <Button type="button" variant="ghost" size="sm" onClick={() => setDraft(message.text)}>
        Edit
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        aria-label="Remove queued message"
        onClick={() => remove.mutate()}
      >
        <X />
      </Button>
    </li>
  );
}

export function QueueList({ agentId, messages }: { agentId: string; messages: QueuedMessage[] }) {
  if (messages.length === 0) return null;
  return (
    <section
      aria-label="Queue"
      className="mx-auto w-full max-w-3xl border-t border-border px-6 py-3"
    >
      <h3 className="mb-2 text-xs font-medium text-muted-foreground">Queued ({messages.length})</h3>
      <ul className="flex flex-col gap-2">
        {messages.map((message) => (
          <QueueRow key={message.id} message={message} agentId={agentId} />
        ))}
      </ul>
    </section>
  );
}

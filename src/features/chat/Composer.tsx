import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowUp, Square } from "lucide-react";
import { type KeyboardEvent, useState } from "react";
import { Button } from "@/components/ui/button";
import { asAgentError } from "@/features/agents/api";
import { MESSAGE_MAX_CHARS, sendMessage, stopTurn } from "./api";

export function Composer({
  agentId,
  running,
  runningTurnId,
}: {
  agentId: string;
  running: boolean;
  runningTurnId: string | null;
}) {
  const client = useQueryClient();
  const [text, setText] = useState("");
  const length = Array.from(text).length;
  const tooLong = length > MESSAGE_MAX_CHARS;
  const send = useMutation({
    mutationFn: () => sendMessage(agentId, text),
    onSuccess: () => {
      setText("");
      void client.invalidateQueries({ queryKey: ["conversation", agentId] });
      void client.invalidateQueries({ queryKey: ["queue", agentId] });
      void client.invalidateQueries({ queryKey: ["agents"] });
    },
  });
  const stop = useMutation({ mutationFn: (turnId: string) => stopTurn(agentId, turnId) });
  const canSend = text.trim().length > 0 && !tooLong && !send.isPending;

  function submit() {
    if (canSend) send.mutate();
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      submit();
    }
  }

  return (
    <div className="border-t border-border p-4">
      {send.isError && (
        <p role="alert" className="mb-2 text-sm text-red-600">
          {asAgentError(send.error).message}
        </p>
      )}
      {running && (
        <p className="mb-2 text-xs text-muted-foreground">
          Messages you send now will wait until the agent finishes.
        </p>
      )}
      <div className="flex items-end gap-2">
        <textarea
          aria-label="Message"
          rows={2}
          value={text}
          placeholder={running ? "Queue a message…" : "Message"}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={onKeyDown}
          className="max-h-48 min-h-14 flex-1 resize-y rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        {running ? (
          <Button
            key="stop"
            type="button"
            variant="secondary"
            aria-label="Stop"
            disabled={stop.isPending || runningTurnId === null}
            onClick={() => runningTurnId && stop.mutate(runningTurnId)}
          >
            <Square />
            Stop
          </Button>
        ) : (
          <Button
            key="send"
            type="button"
            size="icon"
            aria-label="Send"
            disabled={!canSend}
            onClick={submit}
          >
            <ArrowUp />
          </Button>
        )}
      </div>
      {tooLong && (
        <p role="alert" className="mt-1 text-right text-xs text-red-600">
          {length.toLocaleString("en-US")} / {MESSAGE_MAX_CHARS.toLocaleString("en-US")} characters
        </p>
      )}
    </div>
  );
}

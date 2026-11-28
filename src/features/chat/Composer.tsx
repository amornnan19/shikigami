import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowUp, Square } from "lucide-react";
import { type KeyboardEvent, useLayoutEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { asAgentError } from "@/features/agents/api";
import type { GroupMember } from "@/features/groups/api";
import { suggestionAt } from "@/features/groups/mentions";
import { cn } from "@/lib/utils";
import { MESSAGE_MAX_CHARS } from "./api";
import type { ChatScope } from "./scope";

export function Composer({
  scope,
  running,
  runningTurnId,
  blockedReason,
  members,
}: {
  scope: ChatScope;
  running: boolean;
  runningTurnId: string | null;
  blockedReason: string | null;
  members?: GroupMember[];
}) {
  const client = useQueryClient();
  const [text, setText] = useState("");
  const [cursor, setCursor] = useState(0);
  const [highlight, setHighlight] = useState(0);
  const [closedAt, setClosedAt] = useState<number | null>(null);
  const [pendingCursor, setPendingCursor] = useState<number | null>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const found = members ? suggestionAt(text, cursor, members) : null;
  const suggestion = found && found.start !== closedAt ? found : null;
  const active = suggestion ? Math.min(highlight, suggestion.members.length - 1) : 0;

  useLayoutEffect(() => {
    if (pendingCursor === null) return;
    field.current?.setSelectionRange(pendingCursor, pendingCursor);
    setCursor(pendingCursor);
    setPendingCursor(null);
  }, [pendingCursor]);
  const length = Array.from(text).length;
  const tooLong = length > MESSAGE_MAX_CHARS;
  const send = useMutation({
    mutationFn: () => scope.send(text),
    onSuccess: () => {
      setText("");
      setCursor(0);
      void client.invalidateQueries({ queryKey: scope.conversationKey });
      void client.invalidateQueries({ queryKey: scope.queueKey });
      for (const queryKey of scope.refreshKeys) {
        void client.invalidateQueries({ queryKey });
      }
    },
  });
  const stop = useMutation({ mutationFn: (turnId: string) => scope.stop(turnId) });
  const canSend = text.trim().length > 0 && !tooLong && !send.isPending && blockedReason === null;

  function submit() {
    if (canSend) send.mutate();
  }

  function insert(member: GroupMember) {
    if (!suggestion) return;
    const inserted = `@${member.name} `;
    setText(text.slice(0, suggestion.start) + inserted + text.slice(cursor));
    setPendingCursor(suggestion.start + inserted.length);
    setHighlight(0);
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (suggestion && !event.nativeEvent.isComposing) {
      const count = suggestion.members.length;
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setHighlight((active + (event.key === "ArrowDown" ? 1 : count - 1)) % count);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        const member = suggestion.members[active];
        if (member) insert(member);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setClosedAt(suggestion.start);
        return;
      }
    }
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
      <div className="relative flex items-end gap-2">
        {suggestion && (
          <div
            role="listbox"
            aria-label="Members"
            className="absolute bottom-full left-0 z-10 mb-1 flex max-h-56 w-64 flex-col overflow-y-auto rounded-md border border-border bg-background p-1 shadow-md"
          >
            {suggestion.members.map((member, index) => (
              <button
                key={member.agentId}
                type="button"
                role="option"
                aria-selected={index === active}
                tabIndex={-1}
                onMouseDown={(event) => {
                  event.preventDefault();
                  insert(member);
                }}
                className={cn(
                  "flex items-center gap-2 rounded px-2 py-1.5 text-left text-sm outline-none",
                  index === active ? "bg-accent" : "hover:bg-accent",
                )}
              >
                <AgentAvatar name={member.name} color={member.color} className="size-5 text-xs" />
                <span className="truncate">{member.name}</span>
              </button>
            ))}
          </div>
        )}
        <textarea
          ref={field}
          aria-label="Message"
          aria-autocomplete={members ? "list" : undefined}
          rows={2}
          value={text}
          placeholder={blockedReason ?? (running ? "Queue a message…" : "Message")}
          disabled={blockedReason !== null}
          onChange={(event) => {
            setText(event.target.value);
            setCursor(event.target.selectionStart);
            setHighlight(0);
            setClosedAt(null);
          }}
          onSelect={(event) => setCursor(event.currentTarget.selectionStart)}
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

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { asAgentError } from "@/features/agents/api";
import type { ChatScope } from "./scope";

export function ResumeBar({
  scope,
  count,
  interrupted,
}: {
  scope: ChatScope;
  count: number;
  interrupted: boolean;
}) {
  const client = useQueryClient();
  const resume = useMutation({
    mutationFn: () => scope.resume(),
    onSettled: () => {
      void client.invalidateQueries({ queryKey: scope.queueKey });
      void client.invalidateQueries({ queryKey: scope.conversationKey });
      for (const queryKey of scope.refreshKeys) {
        void client.invalidateQueries({ queryKey });
      }
    },
  });
  const waiting =
    count === 1 ? "1 queued message is waiting." : `${count} queued messages are waiting.`;
  return (
    <div
      role="status"
      aria-label="Queue waiting"
      className="mx-auto flex w-full max-w-3xl items-center gap-3 border-t border-border px-6 py-3 text-sm"
    >
      <p className="min-w-0 flex-1">
        {interrupted
          ? `The app closed while the agent was working. ${waiting} Send a message or press Resume to continue.`
          : waiting}
        {resume.isError && (
          <span className="ml-2 text-red-600">{asAgentError(resume.error).message}</span>
        )}
      </p>
      <Button type="button" size="sm" disabled={resume.isPending} onClick={() => resume.mutate()}>
        Resume
      </Button>
    </div>
  );
}

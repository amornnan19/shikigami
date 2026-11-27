import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { asAgentError } from "@/features/agents/api";
import { resumeQueue } from "./api";

export function ResumeBar({
  agentId,
  count,
  interrupted,
}: {
  agentId: string;
  count: number;
  interrupted: boolean;
}) {
  const client = useQueryClient();
  const resume = useMutation({
    mutationFn: () => resumeQueue(agentId),
    onSettled: () => {
      void client.invalidateQueries({ queryKey: ["queue", agentId] });
      void client.invalidateQueries({ queryKey: ["conversation", agentId] });
      void client.invalidateQueries({ queryKey: ["agents"] });
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

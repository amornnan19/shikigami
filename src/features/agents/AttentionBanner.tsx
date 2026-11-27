import { useMutation, useQueryClient } from "@tanstack/react-query";
import { TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CLAUDE_AUTH_KEY } from "@/features/providers/api";
import { type Agent, asAgentError, recreateWorkspace } from "./api";

export const BLOCKING_CODES = ["workspace-missing", "provider-unavailable"];

export function isBlocked(agent: Agent) {
  return agent.attention !== null && BLOCKING_CODES.includes(agent.attention.code);
}

export function AttentionBanner({ agent }: { agent: Agent }) {
  const client = useQueryClient();
  const refresh = () => {
    void client.invalidateQueries({ queryKey: ["agents"] });
    void client.invalidateQueries({ queryKey: CLAUDE_AUTH_KEY });
  };
  const recreate = useMutation({
    mutationFn: () => recreateWorkspace(agent.id),
    onSettled: refresh,
  });
  if (!agent.attention || !isBlocked(agent)) return null;
  const { code, message } = agent.attention;
  return (
    <div
      role="alert"
      aria-label="Agent problem"
      className="flex items-center gap-3 border-b border-border bg-amber-50 px-6 py-3 text-sm text-amber-900"
    >
      <TriangleAlert className="size-4 shrink-0" />
      <div className="min-w-0 flex-1">
        <p>{message}</p>
        {recreate.isError && <p className="text-red-600">{asAgentError(recreate.error).message}</p>}
      </div>
      {code === "workspace-missing" && (
        <Button
          type="button"
          size="sm"
          variant="secondary"
          disabled={recreate.isPending}
          onClick={() => recreate.mutate()}
        >
          Recreate folder
        </Button>
      )}
      {code === "provider-unavailable" && (
        <Button type="button" size="sm" variant="secondary" onClick={refresh}>
          Check again
        </Button>
      )}
    </div>
  );
}

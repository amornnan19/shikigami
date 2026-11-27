import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { AgentAvatar } from "./AgentAvatar";
import type { Agent, AgentState } from "./api";

function StateDot({ state }: { state: AgentState }) {
  if (state === "working") {
    return (
      <span
        role="img"
        aria-label="Working"
        className="size-2 shrink-0 animate-pulse rounded-full bg-green-500"
      />
    );
  }
  if (state === "needs-attention") {
    return (
      <span
        role="img"
        aria-label="Needs attention"
        className="size-2 shrink-0 rounded-full bg-amber-500"
      />
    );
  }
  return null;
}

export function AgentSidebar({
  agents,
  selectedId,
  onSelect,
  onNew,
}: {
  agents: Agent[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
}) {
  return (
    <aside
      aria-label="Agents"
      className="flex h-screen w-60 shrink-0 flex-col gap-3 border-r border-border bg-muted/40 p-3"
    >
      <h1 className="px-2 pt-1 text-lg font-semibold">Shikigami</h1>
      <Button variant="outline" size="sm" onClick={onNew}>
        <Plus />
        New agent
      </Button>
      <ul className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto">
        {agents.map((agent) => (
          <li key={agent.id}>
            <button
              type="button"
              aria-current={agent.id === selectedId ? "true" : undefined}
              onClick={() => onSelect(agent.id)}
              className={cn(
                "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring",
                agent.id === selectedId && "bg-accent font-medium",
              )}
            >
              <AgentAvatar name={agent.name} color={agent.color} />
              <span className="min-w-0 flex-1 truncate">{agent.name}</span>
              <StateDot state={agent.state} />
            </button>
          </li>
        ))}
      </ul>
    </aside>
  );
}

import { AgentAvatar } from "./AgentAvatar";
import type { Agent } from "./api";

export function AgentView({ agent }: { agent: Agent }) {
  return (
    <section aria-label="Agent" className="flex h-screen min-w-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b border-border px-6 py-4">
        <AgentAvatar name={agent.name} color={agent.color} className="size-9" />
        <h2 className="truncate text-lg font-semibold">{agent.name}</h2>
      </header>
      <div className="flex flex-1 items-center justify-center text-muted-foreground">
        No messages yet.
      </div>
    </section>
  );
}

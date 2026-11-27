import { Composer } from "@/features/chat/Composer";
import { QueueList } from "@/features/chat/QueueList";
import { ResumeBar } from "@/features/chat/ResumeBar";
import { Transcript } from "@/features/chat/Transcript";
import { useConversation, useQueue } from "@/features/chat/useConversation";
import { AgentAvatar } from "./AgentAvatar";
import { AttentionBanner, isBlocked } from "./AttentionBanner";
import type { Agent } from "./api";

export function AgentView({ agent }: { agent: Agent }) {
  const { query, loadOlder, loadingOlder } = useConversation(agent.id);
  const queue = useQueue(agent.id);
  const turns = query.data?.turns ?? [];
  const last = turns.at(-1);
  const runningTurnId = last?.status === "running" ? last.id : null;
  const running = agent.state === "working" || runningTurnId !== null;
  const blockedReason = isBlocked(agent) ? (agent.attention?.message ?? null) : null;
  const waiting = queue.data?.length ?? 0;

  return (
    <section aria-label="Agent" className="flex h-screen min-w-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b border-border px-6 py-4">
        <AgentAvatar name={agent.name} color={agent.color} className="size-9" />
        <h2 className="truncate text-lg font-semibold">{agent.name}</h2>
        <span className="text-sm text-muted-foreground">{agent.model}</span>
      </header>
      <AttentionBanner agent={agent} />
      {query.isError && (
        <p role="alert" className="px-6 py-3 text-sm text-red-600">
          Could not load the conversation.
        </p>
      )}
      <Transcript
        turns={turns}
        hasMore={query.data?.hasMore ?? false}
        loadingOlder={loadingOlder}
        loaded={query.isSuccess}
        onLoadOlder={loadOlder}
      />
      <QueueList agentId={agent.id} messages={queue.data ?? []} />
      {waiting > 0 && !running && blockedReason === null && (
        <ResumeBar
          agentId={agent.id}
          count={waiting}
          interrupted={agent.attention?.code === "last-turn-interrupted"}
        />
      )}
      <Composer
        agentId={agent.id}
        running={running}
        runningTurnId={runningTurnId}
        blockedReason={blockedReason}
      />
    </section>
  );
}

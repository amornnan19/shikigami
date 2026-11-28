import { Settings } from "lucide-react";
import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import type { Step } from "@/features/chat/api";
import { Composer } from "@/features/chat/Composer";
import { QueueList } from "@/features/chat/QueueList";
import { ResumeBar } from "@/features/chat/ResumeBar";
import { agentScope } from "@/features/chat/scope";
import { Transcript } from "@/features/chat/Transcript";
import { useConversation, useQueue } from "@/features/chat/useConversation";
import { AUTH_MODE_LABELS } from "@/features/providers/api";
import { FigureTray } from "@/features/shikigami/FigureTray";
import { AgentAvatar } from "./AgentAvatar";
import { AgentSettingsDialog } from "./AgentSettingsDialog";
import { AttentionBanner, isBlocked } from "./AttentionBanner";
import type { Agent, Deleted } from "./api";

export function AgentView({
  agent,
  onDeleted,
}: {
  agent: Agent;
  onDeleted: (outcome: Deleted, askedForTrash: boolean) => void;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const scope = useMemo(() => agentScope(agent.id), [agent.id]);
  const { query, loadOlder, loadingOlder } = useConversation(scope);
  const queue = useQueue(scope);
  const turns = query.data?.turns ?? [];
  const last = turns.at(-1);
  const runningTurnId = last?.status === "running" ? last.id : null;
  const running = agent.state === "working" || runningTurnId !== null;
  const blockedReason = isBlocked(agent) ? (agent.attention?.message ?? null) : null;
  const waiting = queue.data?.length ?? 0;

  return (
    <section aria-label="Agent" className="flex h-screen min-w-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b border-border px-6 py-4">
        <span data-figure-anchor="" className="relative shrink-0">
          <FigureTray
            steps={(last?.items ?? []).filter((item): item is Step => item.type === "step")}
            color={agent.color}
            anchor={36}
          />
          <span className="relative z-10 block">
            <AgentAvatar name={agent.name} color={agent.color} className="size-9" />
          </span>
        </span>
        <h2 className="truncate text-lg font-semibold">{agent.name}</h2>
        <span className="text-sm text-muted-foreground">{agent.model}</span>
        <span className="rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground">
          {AUTH_MODE_LABELS[agent.effectiveAuthMode]}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Agent settings"
          className="ml-auto"
          onClick={() => setSettingsOpen(true)}
        >
          <Settings />
        </Button>
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
        color={agent.color}
      />
      <QueueList scope={scope} messages={queue.data ?? []} />
      {waiting > 0 && !running && blockedReason === null && (
        <ResumeBar
          scope={scope}
          count={waiting}
          interrupted={agent.attention?.code === "last-turn-interrupted"}
        />
      )}
      <Composer
        scope={scope}
        running={running}
        runningTurnId={runningTurnId}
        blockedReason={blockedReason}
      />
      <AgentSettingsDialog
        agent={agent}
        running={running}
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        onDeleted={onDeleted}
      />
    </section>
  );
}

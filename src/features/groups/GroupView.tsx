import { Settings, TriangleAlert } from "lucide-react";
import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import type { Agent } from "@/features/agents/api";
import { Composer } from "@/features/chat/Composer";
import { QueueList } from "@/features/chat/QueueList";
import { ResumeBar } from "@/features/chat/ResumeBar";
import { groupScope } from "@/features/chat/scope";
import { Transcript } from "@/features/chat/Transcript";
import { useConversation, useQueue } from "@/features/chat/useConversation";
import type { Group } from "./api";
import { GroupAvatars } from "./GroupAvatars";
import { GroupSettingsDialog } from "./GroupSettingsDialog";

export function GroupView({
  group,
  agents,
  notice,
  onDeleted,
}: {
  group: Group;
  agents: Agent[];
  notice: string | null;
  onDeleted: () => void;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const lead = group.members.find((member) => member.agentId === group.leadAgentId);
  const scope = useMemo(() => groupScope(group.id), [group.id]);
  const { query, loadOlder, loadingOlder } = useConversation(scope);
  const queue = useQueue(scope);
  const turns = query.data?.turns ?? [];
  const last = turns.at(-1);
  const runningTurnId = last?.status === "running" ? last.id : null;
  const running = runningTurnId !== null;
  const blockedReason = group.attention?.message ?? null;
  const waiting = queue.data?.length ?? 0;
  return (
    <section aria-label="Group" className="flex h-screen min-w-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b border-border px-6 py-4">
        <GroupAvatars members={group.members} limit={5} className="size-8 text-sm" />
        <div className="flex min-w-0 flex-col">
          <h2 className="truncate text-lg font-semibold">{group.name}</h2>
          <span
            id="group-header-folder"
            className="truncate font-mono text-xs text-muted-foreground"
          >
            {group.folderPath}
          </span>
          <span id="group-header-lead" className="text-xs text-muted-foreground">
            {lead ? `Lead: ${lead.name}` : "No lead"}
          </span>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Group settings"
          className="ml-auto"
          onClick={() => setSettingsOpen(true)}
        >
          <Settings />
        </Button>
      </header>
      {group.attention && (
        <div
          role="alert"
          aria-label="Group problem"
          className="flex items-center gap-3 border-b border-border bg-amber-50 px-6 py-3 text-sm text-amber-900"
        >
          <TriangleAlert className="size-4 shrink-0" />
          <p>{group.attention.message}</p>
        </div>
      )}
      {notice && (
        <p role="status" className="border-b border-border px-6 py-3 text-sm text-amber-800">
          {notice}
        </p>
      )}
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
        members={group.members}
      />
      <QueueList scope={scope} messages={queue.data ?? []} />
      {waiting > 0 && !running && blockedReason === null && (
        <ResumeBar scope={scope} count={waiting} interrupted={last?.status === "interrupted"} />
      )}
      <Composer
        scope={scope}
        running={running}
        runningTurnId={runningTurnId}
        blockedReason={blockedReason}
      />
      <GroupSettingsDialog
        group={group}
        agents={agents}
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        onDeleted={onDeleted}
      />
    </section>
  );
}
